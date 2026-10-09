# Rasterwarp: Video Inputs: Design

Date: 2026-10-08
Status: Approved 2026-10-08 (design reviewed section by section in chat)
Builds on: `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md` and `docs/superpowers/specs/2026-10-07-rasterwarp-save-load-design.md` (both on `main`). The motion/output/look spec listed "video backgrounds and live camera" under Later.

## Goals

- **Video files and cameras as inputs:** either one can be the **source** (the image the warp, raster and colorizer manipulate) or the **background** (the color image keyed levels show), alongside today's images.
- **Play modes for clips:** loop forwards or backwards at a speed, ping-pong, or scrub, with the frame position as a parameter that A/B transitions and sequence cues ramp through.
- **Between-frame interpolation:** when the playhead sits between two frames, show the nearest one or blend the two. Optical flow is a later choice in the same setting.
- **Time effects:** a **delay** for cameras (show the camera as it was some seconds ago) and **slit-scan** for clips and cameras (each pixel shows a different moment, chosen line by line, column by column, or by a black-and-white map image).

**Out of scope** (next or later):
- **Optical-flow interpolation:** the next spec. This design leaves its slot: a third `Between frames` choice, computed in the same GPU pass.
- **Streaming long clips** from disk. Clips here are short loops held in memory.
- **Audio** from video files or cameras.
- **Play modes on the camera buffer:** loop, ping-pong or scrub over the last N seconds. Echo (several delayed copies mixed).
- Video as a slit-scan map. Camera format or resolution choice (the device's default mode is used).

## Inputs

The source and the background each have one **input kind**:
- **Image:** today's behaviour (the source falls back to the built-in test card).
- **Video:** a clip file, decoded whole into memory.
- **Camera:** a live DirectShow device, with a rolling buffer of recent frames.

The **source** is always grayscale (luma), so a color clip or camera becomes grayscale before the warp, raster and colorizer see it. The **background** stays full color (RGBA) and keeps its "cover" fit.

**Choosing an input:** the Source and Background sections each have an **Image / Video / Camera** choice. Image and Video have **Open…**; Camera has a device dropdown listing the DirectShow video devices (e.g. "Logi C270 HD WebCam"). Dropping a video file on the window makes it the source, as dropping an image does today. A file counts as an image when the `image` crate recognizes its extension (`image::ImageFormat::from_path`), and as a video otherwise.

## Clips

- **Decoding** uses the linked FFmpeg libraries (`ffmpeg-next` `format` + `codec` + `software-scaling`, already enabled for capture). The first video stream is decoded; audio is ignored.
- **The whole clip is decoded into memory** when it opens, so every frame is instantly available forwards, backwards and when scrubbing.
- **Frames are scaled** to fit inside the canvas size at the time of loading (keeping the clip's aspect ratio; never scaled up), and stored as luma (1 byte per pixel) for the source or RGBA for the background. Each frame also keeps a small copy at the off-air preview's width for the preview (see "The off-air preview"). Changing the canvas size later does not re-decode; the frames are sampled at whatever size they have.
- **The clip records** its frame count and frame rate (the stream's average frame rate; 30 fps if FFmpeg reports none).
- **Memory budget:** clips and camera buffers together may hold at most **2 GB** of frames. A clip that would exceed what's left is refused with its size: "flip.mp4 needs 3.1 GB of memory; shorten it or lower the canvas size".
- **Loading runs on a worker thread.** The panel shows "Loading flip.mp4… 40%". The previous input stays on screen until the clip is ready. Opening another file cancels a load in progress.

## Cameras

- **Opening:** `avdevice` with the `dshow` input format and the device's default mode. This needs `ffmpeg-next`'s `device` feature (the FFmpeg build in `FFMPEG_DIR` includes `avdevice`).
- **A worker thread** reads and decodes frames, scales them to fit the canvas size (as for clips, plus the preview-width copy), and pushes them into the camera's **rolling buffer**: a ring holding the last **buffer length** seconds (a per-camera setting, 1–30 s, default 10 s). Each frame is stamped with its arrival time. The ring's memory counts against the 2 GB budget; a buffer length that doesn't fit is reduced to what does, and the panel says so.
- **The render loop never waits** for the camera; it reads whatever is in the ring.
- **Timing:** a camera follows wall-clock time, also during offline recording (each canvas frame takes the frames that have arrived by then). Offline recordings with a camera are therefore not frame-exact; clips are.

## Playback settings (in the banks)

`Params` gets two `VideoParams`, `source_video` and `background_video`. They are saved with presets, banks and cues, and ramp like other parameters:

| Field | Values | Default | During a ramp |
|---|---|---|---|
| `mode` | Loop, Ping-pong, Scrub | Loop | switches at the midpoint |
| `speed` | −4…+4 (× the clip's own rate; negative plays backwards) | 1 | lerps (through the video clock) |
| `position` | 0–1 of the clip (Scrub) | 0 | lerps |
| `between` | Nearest, Blend (Optical flow later) | Blend | switches at the midpoint |
| `delay` | 0–30 s (cameras; clamped to the camera's buffer length when used) | 0 | lerps |
| `slit` | Off, Rows, Columns, Map | Off | switches at the midpoint |
| `slit_depth` | 0…max depth, seconds | 1 | lerps |
| `slit_flip` | on/off | off | switches at the midpoint |

All of these settings are ignored for an image input. A camera ignores `mode`, `speed` and `position` (it is always live, minus `delay`); a clip ignores `delay`.

## The playhead

The playhead turns the settings and time into a **fractional frame index** for each input. That index then gives the two frames to mix (`lo`, `hi`) and a mix weight. This is pure maths in `video/playhead.rs`, tested without a GPU or files.

- **Video clock:** `Clocks` gets `video: [f64; 2]`, an unwrapped running total of clip time in seconds, one per input. It advances by `speed × dt` each canvas frame (so playback follows the canvas frame clock, and offline recording is frame-exact). It is handled during ramps the same way the oscillator phase clocks are, so ramping speed (e.g. 1× to −2×) stays smooth.
- **Loop:** frame index = clock × clip fps, wrapped into `[0, count)`. Blending at the wrap mixes the last frame with the first.
- **Ping-pong:** the same index folded back and forth over `[0, count − 1]`. The end frames are not doubled.
- **Scrub:** frame index = `position × (count − 1)`. The video clock is ignored.
- **New input:** opening a clip resets its video clock to 0 (the first frame).
- **Mode switches:** when the effective mode changes from Scrub to Loop or Ping-pong (a cut, or the midpoint of a ramp), the video clock is set so playback continues from the frame on screen.
- **Camera:** the playhead is "newest arrival time − `delay`". The frame index comes from the arrival stamps, interpolating between the two frames either side. If that time is older than the buffer holds, it clamps to the oldest frame.
- **Nearest** rounds the index to a whole frame. **Blend** mixes `lo` and `hi` by the fractional part.

## Slit-scan

Each pixel shows the input from a different moment: `offset(x, y) = slit_depth × map(x, y)` seconds behind the playhead.

- **Maps:** `map` runs from 0 (now) to 1 (`slit_depth` ago).
  - **Rows:** `map = y`; the top row is now, the bottom row is `slit_depth` ago.
  - **Columns:** `map = x`; the left column is now.
  - **Map:** a black-and-white image, stretched over the frame. Black is now, white is `slit_depth` ago, and gray steps lie in between. It is loaded as grayscale (any image the `image` crate opens) and sampled with linear filtering.
  - **Flip** uses `1 − map` for any of them.
- **"Behind the playhead":**
  - **Loop and Ping-pong:** subtract the offset (× clip fps) from the unwrapped clock's frame index in the direction of travel (`sign(speed)`, with speed 0 counting as forward), then wrap or fold it like the playhead.
  - **Scrub:** subtract from the position's frame index, clamped at frame 0.
  - **Camera:** subtract from the delayed playhead time, clamped to the oldest frame in the buffer (so `delay + slit_depth` beyond the buffer length shows the oldest frame).
- **`between` applies per pixel:** Blend gives smooth gradients through time; Nearest gives visible stepped bands.
- **The map image path belongs to the input** (like the background path), not to the banks; projects save it. With no map image loaded, Map behaves as Rows.

## On the GPU

- **A new pass, `passes/frames.rs`**, runs before warp/raster for the source and before composite for the background. It produces the texture the rest of the pipeline already samples: `R8Unorm` for the source, `Rgba8UnormSrgb` for the background. It mixes the `lo` and `hi` frames, or for slit-scan computes each pixel's own frame index and mixes per pixel. Optical flow will be computed here later.
- **Frames live in a texture array used as a ring:** frame index *n* lives in layer `n mod layers`, and only frames missing from the needed window are uploaded. With slit-scan off, the window is just `lo` and `hi`, so normal playback uploads at most one new frame per input per canvas frame when playing forwards.
- **Window and depth cap:** with slit-scan on, the window is `ceil(slit_depth × fps) + 2` frames. Layers are capped by the smaller of 256 (wgpu's default `max_texture_array_layers`), the device limit, and a **1 GB VRAM budget** divided by the bytes per layer. The panel shows the resulting maximum depth (about 8.5 s for 720p grayscale at 30 fps; less for full-color 1080p backgrounds), and the depth slider stops there. A cue or file with a deeper value is clamped when used.
- **Image inputs** keep today's single texture, so an image costs nothing new.
- **Raster beam reset:** changing the input (new file, new camera) resets the beam as changing the source image does today. New frames from the same input do not.

## The off-air preview

The preview runs its own playheads from the off-air bank's (or cue's) settings, using the same frames in memory, so the off-air look shows its own position, speed and slit-scan. It reads the frames' preview-width copies and keeps its own small texture rings at preview size, so its VRAM cost is small.

## The panel

The Source and Background sections each get:
- The **Image / Video / Camera** choice, with **Open…** or the device dropdown, and the file or device name with its size and frame rate ("flip.mp4: 1280×736, 107 frames at 24 fps").
- A loading line while a clip loads, and a camera's **Buffer** length.
- A **video group** (hidden for images):
  - **Mode:** Loop / Ping-pong / Scrub, with **Speed** (Loop, Ping-pong) or **Position** (Scrub).
  - **Between frames:** Nearest / Blend.
  - **Delay** (cameras).
  - **Slit-scan:** Off / Rows / Columns / Map, with **Depth** (showing its maximum), **Flip**, and **Map image…** / **Clear**.

The video group edits the same bank or cue the rest of the panel edits (Live: on screen; Transition: the off-air bank; Sequence: the selected cue).

## Saving

- **Presets, banks and cues** carry `source_video` and `background_video` through `Params`. Older files load with the defaults (Loop, 1×, Blend, no delay, slit-scan off).
- **Projects** keep the existing `source` and `background` path fields, which now hold either an image or a video file (told apart by extension, as when opening). New optional fields:
  - `source_camera` / `background_camera`: the device name and buffer length. When set, the camera is the input and the path field is ignored.
  - `source_slit_map` / `background_slit_map`: the map image paths.
- The file `version` stays 1: every addition is a new optional field, which the tolerant rules already handle.
- **Not saved:** the video clocks (an opened project starts every clip at its first frame, or at `position` in Scrub) and camera buffers.

## Errors

Every error is shown in the panel, and the previous input stays on screen.
- **A file that won't open,** or has no video stream: "Could not open clip.mp4: no video stream".
- **A clip over the memory budget,** with the memory it would need (see "Clips").
- **A missing camera** when opening a project or choosing a device: the error names the device, and the input falls back to no input (the test card for the source, black for the background).
- **A camera that disconnects:** the last frame stays on screen with **"No Signal"** in the panel, and the camera is retried every few seconds until it returns or another input is chosen.
- **A missing or unreadable map image:** reported, and Map behaves as Rows.
- **A project whose video file is missing** is reported like a missing image today.

## Code structure

- `src/video/mod.rs`: the input kinds, the frame store interface (frame *n* or the frames around time *t*, plus the preview copy), and the shared memory budget.
- `src/video/clip.rs`: decoding a file into memory on a worker thread, with progress and cancellation.
- `src/video/camera.rs`: listing devices, the capture thread, and the rolling buffer.
- `src/video/playhead.rs`: the pure playhead, slit-offset and ring-layer maths.
- `src/passes/frames.rs`: the texture-array ring and the mix and slit-scan pass.
- `src/params.rs` / `src/blend.rs`: `VideoParams`, its ranges and clamping, and blending it (with `Clocks.video`).
- `src/session.rs`: the new project fields.
- `src/app.rs`: opening and switching inputs, driving the playheads each canvas frame, and errors.
- A new `src/inputs_ui.rs` for the Source and Background sections, so `ui.rs` doesn't grow further (as `files_ui.rs` did for files).
- `Cargo.toml`: `ffmpeg-next` gains the `device` feature.

## Testing

- **Playhead (unit):** Loop wraps both ways, Ping-pong folds without doubling the end frames, signed speed, Scrub mapping, Nearest vs Blend weights, the Scrub-to-Loop hand-over, camera delay lookup by arrival time and clamping to the buffer, slit offsets for Rows, Columns and Flip, ring layers and the depth cap.
- **Params (unit):** `VideoParams` defaults for old files, clamping, and blending (lerp vs midpoint switch).
- **Projects (unit):** round trip of the new fields, and old projects loading unchanged.
- **Clip decoding:** `samples/anthony.mp4` opens with 60 frames at 24 fps, and the luma and RGBA frames have the scaled size; the memory budget refuses an oversized clip; an unreadable file reports an error.
- **GPU smoke tests:** the frames pass at mix weights 0, 0.5 and 1; slit-scan Rows over a sequence of flat frames (frame *n* has brightness *n*) gives a top-to-bottom gradient through time.
- **Camera:** a test that opens the first camera and receives a frame, which skips itself when no camera is present.
- **By hand:** the samples as source and background in each mode, a Transition scrubbing between two positions, the C270 with delay and slit-scan, and unplugging the camera.

## Later

- **Optical-flow interpolation** (next spec): a third `Between frames` choice. Flow between neighbouring clip frames is computed once on the GPU and cached; frames are warped toward the playhead's fraction from both sides and blended, falling back to blending where the two flows disagree.
- Play modes and echo on the camera buffer; video as a slit-scan map; streaming long clips; audio; camera mode and resolution choice.
