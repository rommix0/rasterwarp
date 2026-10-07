# Rasterwarp: Motion, Output and Look: Design

Date: 2026-10-07
Status: Approved 2026-10-07; amended 2026-10-07 with the off-air preview (Plan 2)
Builds on: `docs/superpowers/specs/2026-10-06-rasterwarp-prototype-design.md` (the prototype, now on `main`)
Research: `docs/research/scanimate-manuals/` (notes taken from the three scanned manuals in `manuals/`)

## Goals

Six features, delivered as three implementation plans. Each plan leaves the app working.

| Plan | Features |
|---|---|
| 1. Motion | A/B transition mode with drawn curves; sequence ramps; oscillator extras (Frame/Free sync, sine/cosine slave, ramp envelope) |
| 2. Output | Off-air preview overlay; configurable canvas resolution; video capture (HEVC NVENC 4:4:4 or FFV1) via linked FFmpeg libraries, in real-time or frame-accurate offline mode |
| 3. Look | True raster (scanline) mode; colorizer thresholds; edge fringing; rotation axis wander; level keying over a background image |

**Out of scope** (recorded in "Later"): raster sections, sequential intensity, blanking wipes, the node graph and modulation matrix, MIDI/OSC, audio-driven "mouth control", alpha-channel export, video backgrounds, overlapping sequence ramps.

## Grounding in the original hardware

These findings from the manuals shape the design. Page references are in the research notes.

- **Two keyframes and a ramp.** The 1969 Scanimate is an INITIAL/FINAL machine. Throwing the INITIAL-FINAL switch starts a linear animation ramp that moves position, size, depth and rotation. RATE pots set its speed.
  - Later Animation Aid modules add up to 5 sequence ramps. Each has a rate pot, a **linear or sine (S-curve) shape**, and a start time set on a 3-digit frame thumbwheel (0–999 frames, ×2 range, counted at 24 fps in the examples).
  - Resetting a ramp is near-instant.
- **The raster is the image.** The display is a 600-line, 48 Hz, non-interlaced raster that is itself deflected.
  - Enlarging spreads the lines apart into visible gaps, and shrinking packs them together.
  - Rotation rotates the lines with the image.
  - An intensity compensator raises beam brightness roughly in proportion to area, so a spread raster doesn't dim.
- **Oscillators.** Triangle cores with a diode sine shaper, from sub-Hz up to about 500 Hz (LF) or about 90 kHz line-locked (HF, "raster bending").
  - **Sync modes:** FRAME gives a stationary ripple, and OFF lets the waves drift.
  - Oscillator 4 can be slaved to oscillator 3 with a 90° offset, for rolls, circles and figure-8s.
  - **Ramp Select** (MAX→MIN, MIN→MAX, CONSTANT) is the only envelope.
- **Colorizer.** Four threshold pots split brightness into 5 hard-banded levels, each with its own R/G/B, and any level can be keyed over background video.
  - Vertical edges show colored fringing from the scan-converter beam's finite transition time.
  - Imperfect multiplier nulling makes the rotation axis wander.

---

## Plan 1: Motion

### Modes

`Mode` is one of `Live`, `Transition` or `Sequence`, chosen in the panel.

- **Live:** the panel edits the on-air parameters directly. This is today's behavior.
- **Transition (A/B):** there are two banks, `A` and `B`, with one on air.
  - On entering this mode, the off-air bank is set to a copy of the on-air bank, so nothing jumps.
  - The panel edits **only the off-air bank**, and the output keeps showing the on-air bank.
- **Sequence:** up to 5 cues, described below. While the sequence is stopped, the panel edits the selected cue and the output shows it.

### Transition control (A/B)

- **Transition** button, plus the **Space** key when egui doesn't want the keyboard: start a ramp from the on-air bank to the off-air bank.
  - **Duration:** 0.1–30 s, default 2.0 s.
  - **Curve:** chosen from the curve library (see Curves).
  - When the ramp completes, the destination becomes on air and the panel now edits the other bank.
- Pressing Transition **while a ramp is running reverses it** from its current progress, toward the bank it came from, over the remaining proportion of the duration. This works like pulling a T-bar back.
- **Cut:** swaps on-air and off-air instantly. If a ramp is running, it is cancelled and the destination goes on air.
- A progress bar shows the ramp's progress (0–1).
- Ramps run on animation time, so **Pause** freezes them.

### Curves

- **Built-in curves:**
  - `Linear`: y = x.
  - `S-curve`: y = 0.5 − 0.5·cos(πx), the manual's sine ramp.
- **Custom curves:** a named list of control points `(x, y)`.
  - The endpoints `(0,0)` and `(1,1)` are fixed.
  - Interior points have x strictly inside (0,1), and y is clamped to [−0.5, 1.5] so overshoot and anticipation are possible.
  - Evaluation uses monotone cubic Hermite interpolation (Fritsch–Carlson) through the points sorted by x. This means no wobble between points; overshoot comes only from points placed outside [0,1].
- **Curve library:** the built-in curves plus up to 8 user curves. User curves are added, renamed and deleted in the panel and last for the session only (no file persistence yet).
- **Curve editor** (an egui painter widget, about 260×160 px):
  - Draws the curve, a grid, and the 0 and 1 lines.
  - Drag interior points to move them; double-click adds a point; right-click removes one.
  - A transition and each sequence cue hold a curve *reference* (built-in or a user-curve index). Editing a user curve affects everything that uses it.

### Blending (`blend.rs`)

`blend(a: &Params, b: &Params, t: f32, envelope_phase: Option<f32>) -> FrameParams`, where `t` is the curve-mapped progress. `FrameParams` is what the renderer consumes. It holds the global fields and up to **8 weighted oscillator slots**.

- **Numeric fields** (zoom, rotation, offsets, drift, softness, cycle speed, feedback, glow, raster numerics, thresholds, bandwidth, axis wander): `lerp(a, b, t)`. `t` may lie outside [0,1] for overshooting curves. Each result is clamped to its parameter range.
- **Palette colors:** converted to linear, lerped, then clamped to [0,1].
- **Discrete fields:**
  - Colorizer level count, bypass, raster enabled, raster line count, key enabled and key level mask all switch at `t >= 0.5`.
  - The slave flag is resolved per bank before blending.
- **Oscillators**, compared pairwise by index i:
  - If A[i] and B[i] have the same waveform, target, input and sync, one slot is used. Its numeric fields (frequency, amplitude, phase, phase speed, LFO rate and depth) are lerped and its weight is 1.
  - Otherwise two slots are used: A[i]'s settings with weight `1 − t`, and B[i]'s with weight `t`.
  - `t` is clamped to [0,1] for these weights.
  - The total is never more than 8 slots.
- **Frame-sync phase continuity:** each oscillator's phase term is `phase + phase_speed · time` for Free sync and `phase` for Frame sync. Lerping phase speeds could make the wave jump. To avoid that, the phase-speed contribution is kept as an *accumulated phase* per bank oscillator, which the app advances each frame by `phase_speed · dt`, and the accumulated phases are lerped.
- **No transition running:** `blend(on_air, on_air, 0.0, None)`.

### Oscillator extras

- **Sync** (`Free`, the default, or `Frame`): Free advances the accumulated phase by `phase_speed · dt`. Frame leaves it unchanged, so the ripple stands still and `phase` is the only phase control. Drift noise applies in both modes.
- **Slave 4 → 3:** a per-bank flag. When it's on, oscillator 4's waveform, frequency, input, sync, phase speed and LFO come from oscillator 3, and its phase = oscillator 3's phase + 0.25 cycle. Its own target and amplitude are kept. The panel greys out the copied controls.
- **Ramp envelope** (`Constant`, the default, or `Swell`):
  - A Swell oscillator's amplitude is multiplied by `sin(π·p)`, where `p` is the running ramp's raw linear progress (0 → 1), and by 0 when no ramp is running.
  - So it is still at rest and wobbles only while a transition or sequence ramp is moving: the classic "wiggle in transit".
  - **This replaces the approved Fade in / Fade out / Constant set.** In an A/B model a fade-out leaves the oscillator silent or jumping when the destination bank goes on air. Fading in or out is already done by setting that oscillator's amplitude to 0 in the destination bank.

### Sequence mode (`sequence.rs`)

- Up to **5 cues**. Each cue holds:
  - `params: Params` (a full snapshot);
  - `start_frame: u32`, 0–999, at 24 fps (seconds shown alongside);
  - `duration_frames: u32`, 1–999;
  - a curve reference.
- **Creating and editing cues:**
  - On first entering Sequence mode, the sequence has one cue, a copy of the current on-air parameters. The cues are kept when you leave and come back (as built in Plan 1).
  - **Add cue** appends a copy of the selected cue, up to 5. **Delete cue** removes the selected one, but at least one cue always remains.
  - While stopped, the panel edits the **selected cue** directly, and the output shows it.
  - Cue 1's start frame is always 0. Cues are kept sorted by start frame, and each start frame must be greater than the previous cue's.
- **Leaving Sequence mode:** the currently displayed cue's parameters become the on-air bank for Live and Transition modes.
- **Run:** resets the frame counter to 0 and starts from cue 1's parameters. When the counter reaches cue k's start frame (k ≥ 2), a ramp starts from the current state to cue k, using cue k's duration and curve.
- **Overlapping cues:** if cue k+1's start frame arrives while cue k's ramp is still running, cue k snaps to its end first. Real hardware could overlap ramps, but that needs a patch matrix (later).
- **Stop:** freezes the counter. **Reset:** instantly shows cue 1 with the counter at 0.
- **Loop:** when the last ramp finishes and the counter passes the last cue's end, return to cue 1 (an instant reset) and run again.
- The counter is driven by animation time, so Pause freezes it.

### Renderer interface change

`Renderer::render(.., frame: &FrameParams, time: f32, ..)` replaces `&Params`. `WarpUniforms` grows to 8 oscillator slots plus a slot count, and `OscUniform.lfo.z` carries the slot weight.

---

## Plan 2: Output

### Off-air preview

A small live view of what you're editing but isn't on air yet, in the bottom-right corner of the window.

- **What it shows:** `Motion::preview() -> Option<(FrameParams, PreviewLabel)>`.
  - **Transition mode:** the off-air bank ("Off-air: A" / "Off-air: B"), the one the panel edits and Transition would bring in. During a ramp it keeps showing that bank (a forward ramp's destination). When the ramp finishes, it shows the new off-air bank (the one just left).
  - **Sequence mode:** the selected cue ("Cue N") while the sequence is running. When stopped, the output already shows the selected cue, so there's no preview.
  - **Live mode:** no preview.
- **Rendering:** a second instance of the existing `Renderer`, with its own feedback trails and bloom, at a preview size of 480 px wide with the canvas's aspect ratio (480×270 at 16:9; even height, at least 64). It draws into an offscreen `Rgba8UnormSrgb` texture, which egui shows as a framed overlay with the label, anchored in the bottom-right corner of the window with a small margin.
  - The preview's composite uses a fit of (1, 1) and the same CRT settings as the main output.
  - It costs about 6% of a 1080p frame's pixel work.
  - When the canvas aspect ratio changes, the preview resizes to match.
- **Phase clocks:** `Motion` keeps a third clock set, `preview`, advanced each frame with the preview's parameters. The preview's oscillator phases are independent of the on-air output: when a transition starts, the ramp begins from the on-air phases (as before), so the landed wave may sit at a different point in its cycle than the preview showed. Shapes, colors and speeds match. Starting the ramp from the preview's phases instead would make the ramp sweep through every cycle the two clocks drifted apart by.
- **Trails:** the preview's feedback buffers are cleared whenever what it shows changes source: a bank swap or Cut, a different cue selected, or the preview appearing.
- **Controls:** a **Show preview** checkbox (default on). Pause freezes the preview with everything else.
- **Capture:** the preview is never recorded. Capture records the clean main output only.

### Canvas resolution

- **Presets:** 640×480, 854×480, 1280×720, 1920×1080 (default), 2560×1440, 3840×2160, or a custom W×H.
- Values are rounded to even numbers, clamped to at least 64 and at most `max_texture_dimension_2d`, and shown before being applied.
- **Apply:** rebuilds every canvas-sized render target (`Renderer::resize(device, size)`) and clears the trails. It's disabled while recording.
- Bloom level sizes, chroma-bleed scaling and the raster pass follow the canvas size, so the look scales with resolution. The default raster line count stays an independent parameter.

### Capture

- **Panel controls:** a Record/Stop button; a **format** choice (`HEVC NVENC 4:4:4, near-lossless` or `FFV1 lossless`); the capture folder (default `captures/` under the working directory, changeable with an `rfd` folder dialog); and the elapsed time, frames written and frames dropped.
- **Files:**
  - HEVC goes to `rasterwarp-YYYYMMDD-HHMMSS.mp4`. It's written as fragmented MP4 (`movflags=frag_keyframe+empty_moov`), so an interrupted capture is still playable.
  - FFV1 goes to `rasterwarp-YYYYMMDD-HHMMSS.mkv`.
- **Encoder settings:**
  - HEVC: `hevc_nvenc`, `yuv444p`, preset `p4`, tune `hq`, `rc=constqp`, `qp=18`.
  - FFV1: `level=3`, `slices=16`, `slicecrc=1`, `coder=1`, slice threading on, `bgra`, so it's bit-exact.
  - Both run at 60 fps CFR.
  - Measured on the dev machine with the ffmpeg CLI at 1080p60: HEVC NVENC 4:4:4 at 4.6× realtime (49 MB per 10 s); FFV1 at 2.5× realtime (137 MB per 10 s). ProRes was ruled out at 0.7× realtime.
- **What's recorded:** the clean output.
  - Each frame, the composite pass also draws into a canvas-sized `Rgba8UnormSrgb` capture texture, with fit = (1,1), no letterbox and no UI.
  - The texture is copied into one of **3 staging buffers** (rows padded to 256 bytes) and mapped asynchronously.
  - A mapped frame is un-padded and sent through a **bounded channel (capacity 4)** to the encoder thread.
  - If the channel is full, the frame is **dropped and counted**. Rendering never blocks on encoding.
- **Timing:**
  - A frame is captured only when `floor(time · 60)` advances, and its pts is that frame number. Pausing stops the clock, so it also stops new frames.
  - This gives clean 60 fps output whatever the display refresh rate, as long as the display runs at 60 Hz or faster.
  - If rendering runs below 60 fps, frames are missing and the pts gaps make the encoder hold the previous frame. The panel warns about this.
- **Offline (frame-accurate) mode:** a capture-mode choice of **Real-time** (default, as above) or **Offline**.
  - While recording in Offline mode, animation time advances by exactly 1/60 s per rendered frame, ignoring wall-clock time.
  - Every rendered frame is captured, with pts = frame index.
  - Sending a frame to the encoder **blocks when the channel is full**, so no frame is ever dropped. Rendering, and the on-screen preview, slow down to the encoder's pace.
  - Transitions, sequences, oscillator phases and drift all run on that same clock, so the file is exactly what real-time playback would look like at a perfect 60 fps.
  - An optional **Stop after N seconds** field (0 = manual stop) applies to both modes and is mainly useful for offline renders.
  - The panel shows "offline: N× realtime".
  - When the recording stops, the clock returns to wall-clock time from the current animation time, so there's no jump.
- **Encoder thread:** owns the swscale context, encoder and muxer.
  - Frame buffers are owned `Vec<u8>`s sent over the bounded channel and returned for reuse on a second channel.
  - It converts RGBA → `yuv444p` (HEVC, swscale with BT.709) or → `bgra` (FFV1, byte swizzle), then encodes and muxes.
  - On Stop it sends EOF, drains, and writes the trailer.
  - Errors are reported back to the UI over a channel, and the recording stops cleanly.

### FFmpeg integration

A feasibility spike verified everything in this section on the dev machine (2026-10-07).

- **Crate:** `ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }`. It uses ffmpeg-sys-next 9.0.0 and bindgen.
  - It links the shared **FFmpeg 9.0.2** build at `C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared`: libavcodec 63, libavutil 61, MSVC import libs.
- **Build environment:** set in `.cargo/config.toml` `[env]` (machine-specific paths, with a comment):
  - `FFMPEG_DIR`: the shared build directory.
  - `LIBCLANG_PATH`: `C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Tools\Llvm\x64\bin`. The libclang bundled with VS2019 works with bindgen.
  - A cold build of the bindings takes about 45 s.
- **Runtime DLLs:**
  - `build.rs` copies the 5 required DLLs (`avcodec-63`, `avformat-63`, `avutil-61`, `swscale-10`, `swresample-7`; about 121 MB) from `$FFMPEG_DIR/bin` into `target/<profile>/` and `target/<profile>/deps/`, so `cargo run`, `cargo test` and the built exe all find them.
  - A missing DLL makes the exe exit at startup with 0xC0000135 and no message. `build.rs` fails the build with a clear error if any DLL is missing from `FFMPEG_DIR`.
- **Encoder details from the spike:**
  - **Setup order:** set `GLOBAL_HEADER` before open, call `stream.set_parameters(&encoder)` after `open_with`, and read the stream time base after `write_header` so every packet can be rescaled.
  - **HEVC color:**
    - Feed `yuv444p` (RGB-type input silently becomes 4:2:0).
    - Tag BT.709 and limited range.
    - Call `sws_setColorspaceDetails` with the ITU-709 coefficients, because swscale defaults to BT.601.
    - Profile `rext`.
  - **FFV1:**
    - Pixel format `bgra`, because FFV1 rejects 8-bit `gbrp`/`rgb24`, and RGBA→BGRA is a bit-exact swizzle.
    - **Slice threading must be enabled** (`set_threading(Slice, 0)`, `slices=16`), otherwise it encodes at 7.6 fps.
  - **Draining:** `receive_packet` returns `EAGAIN` when it needs more input (not an error) and `Eof` after the flush. Stop must `send_eof`, drain, then `write_trailer`.
  - **Frame strides are padded**, so copy row by row into a reused `AVFrame`.
- **Measured in-process throughput at 1080p** (worst-case noisy content):
  - HEVC NVENC yuv444p: about 215 fps. swscale is the main cost, at about 345 fps single-threaded.
  - FFV1: about 38–45 fps.
  - **FFV1 therefore cannot hold 60 fps at 1080p on grainy content.** It will drop frames, which the panel counts. The format menu labels it "lossless, may drop frames above 720p".
  - **Offline mode** (see Capture) removes this limit: FFV1 and 4K renders never drop frames.
- **NVENC availability:** `find_by_name("hevc_nvenc")` succeeds even without NVIDIA hardware, but `open` fails. This is handled as a capture-start error suggesting FFV1. It also fails if the consumer-GPU session cap is reached.
- **Licensing:** the "full_build" FFmpeg is GPL. Fine for personal use; check the license before distributing binaries.

---

## Plan 3: Look

### True raster mode

- **Params:**
  - `raster.enabled` (default off);
  - `lines` (100–1200, default 600);
  - `beam_width` (0.5–4.0 canvas pixels, default 1.2);
  - `compensation` (0–1, default 1);
  - `speed_compensation` (0–1, default 0.3).
- When enabled, a **raster pass** replaces the warp pass and writes the same grayscale target, so colorize and everything after it is unchanged.
- **Geometry:** `lines` instances. Each is a triangle strip of `segments = clamp(canvas_width / 2, 256, 2048)` quads along the line.
- **Vertex shader:**
  - Each vertex has a source position (u along the line, v = line centre).
  - It is mapped *forward* into the frame by the same deflection functions as the warp pass. Those move into a shared `shaders/deflection.wgsl`, prepended to both passes.
  - The global transform is applied in the forward direction.
  - The vertex is then offset by ±half the beam width along the line's normal, estimated from neighbouring samples.
- **Fragment shader:** brightness = source luminance at (u, v) × a Gaussian beam profile across the line × compensation gain. **Additive blending** into a cleared target, so packed lines overlap and brighten.
- **Compensation gain:**
  - **Area term:** `mix(1, spacing_ratio, compensation)`, where `spacing_ratio` is the local line spacing divided by the rest spacing, found by finite difference of the vertical deflection with respect to v.
  - **Speed term:** `1 + speed_compensation · min(speed / 0.5, 4)`. `speed` is how fast the vertex is moving, in frame heights per second, found by evaluating the deflected position at `time` and `time − 1/60`.
  - The two terms are multiplied. This follows the manual's compensator, which boosted brightness with both raster area and animation speed (|d/dt|), so fast sweeps don't fade.
- **Warp vs raster:** warp is an inverse map and raster is a forward map, so the same oscillator settings give equivalent but mirrored ripples. This is documented in the UI tooltip.
- The composite's painted-on scanline overlay is forced to strength 0 while raster mode is on, because the lines are real.

### Colorizer thresholds

- `thresholds: [f32; 7]`, ascending. Only the first `levels − 1` are used.
- **Default:** even spacing (`k/levels`). An **Even** button restores it.
- **UI:** one slider per active threshold. Each slider is constrained between its neighbours.
- **Shader:** the level index is the number of thresholds at or below g. Softness blends across a threshold within ±softness/2.

### Edge fringing

`colorize.bandwidth`, 0–8 canvas pixels, default 1.0. Before posterizing, the colorize pass blurs the grayscale horizontally with a Gaussian of that σ (7 taps, scaled). Sharp vertical edges then briefly pass through the intermediate levels, so their colors appear as thin fringes. At 0 there is no blur.

### Rotation axis wander

`warp.axis_wander`, 0–1, default 0.15. The rotation pivot moves by `axis_wander · 0.02 · |sin(rotation)| ·` (2D slow value noise of time). Both the warp and raster passes apply it.

### Level keying

- **Params:** `key.enabled` (default off) and `key.levels: u8` (bitmask of see-through levels, default bit 0).
- **Background image:** loaded with an `rfd` file dialog, converted to RGBA, and size-checked with `ensure_fits`. It is drawn with a "cover" fit and is not a `Params` field. Without a background image, keyed areas show black.
- **Pipeline:**
  - Colorize writes alpha = 0 for see-through levels and 1 otherwise (blended across softness).
  - Feedback carries alpha with the same `max` rule.
  - Bloom ignores alpha.
  - Composite output = `background · (1 − α) + image · α + bloom`, then the CRT effects.
- Capture records the composited result. Alpha export is out of scope.

---

## Error handling

- **Capture start failure** (encoder missing or NVENC unavailable, folder not writable, file exists): show the error in the panel, don't start, and keep running. If NVENC is unavailable, the panel suggests FFV1.
- **Encoder failure mid-recording:** stop the recording, finalize what was written where possible, and show the error.
- **Canvas apply failure** (size outside limits after clamping is impossible by construction): the inputs are clamped before applying.
- **Background image load failure:** same rules as source loading (warn, keep the current background, show the error).
- **Curve editor:** points can't cross each other or the endpoints, because x is clamped between neighbours ± 0.01.

## Testing

All pure logic gets unit tests. GPU parts get pipeline-build tests and readback tests.

- `curve.rs`: Linear and S-curve values; endpoints are exact; monotone interpolation never overshoots between in-range points; out-of-range points overshoot; clamping of y; adding and removing points keeps them sorted.
- `transition.rs`: entering Transition copies the on-air bank; start, progress and completion swap the banks; reversing mid-ramp; Cut during and outside a ramp; paused time doesn't advance.
- `blend.rs`: lerp of numerics including t outside [0,1] with clamping; linear-light palette lerp; discrete fields switch at 0.5; matching oscillators produce one slot and mismatched ones two, weights sum to 1, never more than 8 slots; slave-oscillator derivation; Swell multiplier is 0 at rest, 0 at both ramp ends and 1 at the midpoint; accumulated-phase lerp is continuous.
- `sequence.rs`: cues start at their frames; snap-on-overlap; Reset; Loop; Stop freezes.
- `motion.rs` preview: none in Live mode; the off-air bank in Transition mode, the destination during a ramp and the new off-air bank after it finishes; none while a sequence is stopped and the selected cue while it runs; the preview clocks advance with the preview's parameters; a source change is reported so the trails can be cleared.
- `capture`:
  - Pure pieces: file naming; frame decimation from time (60 fps at a 144 Hz display); row-padding removal; ring-slot rotation; drop counting when the channel is full; the offline clock (exactly 1/60 s per frame, every frame captured, return to wall-clock without a jump); Stop after N seconds.
  - Integration test: encode 30 frames with FFV1 into a temp directory, reopen with the FFmpeg library, and check the frame count, size and lossless pixel equality of one frame.
  - The same test with HEVC NVENC, skipped with a message when NVENC is unavailable.
- **GPU readback tests** (extending `tests/smoke.rs`):
  - Raster mode on a flat white source with no deflection and 100 lines: output rows alternate between lit and dark at the expected period.
  - With a vertical stretch, the gaps widen.
  - Colorizer thresholds: a gradient source gives level boundaries at the threshold positions.
  - Keying: see-through level pixels show the background color.
  - Canvas resize: rendering after `resize` produces the new size.
  - Preview: a second `Renderer` at 480×270 renders into an offscreen texture of that size.
- **Manual (release build):** transitions, curve editor, sequence playback, recording 30 s in each format, then playing the files in a media player; raster mode look; keying with a background image.

## Later (documented, not built now)

Raster sections (up to 5 bands with independent deflection and intensity); the sequential intensity generator; blanking-comparator wipes; modulation matrix / node graph (multipliers, rectifiers, summers, programmed phase lock); MIDI/OSC; audio-driven amplitude ("mouth control"); alpha export (FFV1 with alpha); video backgrounds and live camera; overlapping sequence ramps; persisting curves, cues and presets to disk.
