# Rasterwarp Video Inputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Video files and DirectShow cameras as the source or the background, with play modes (Loop, Ping-pong, Scrub) in the banks, nearest-or-blended frames, camera delay, and slit-scan (rows, columns or a map image).

**Architecture:** Clips are decoded whole into memory (and cameras into a rolling buffer) on worker threads, as tightly packed luma or RGBA frames plus a small copy for the off-air preview. Each canvas frame, a pure playhead turns the bank's `VideoParams` and the video clock into a `Sample` (a fractional frame index, a slit-scan reach and a window of frames); a new frames pass keeps that window in a GPU texture-array ring and draws the input's picture, which the existing pipeline samples like an image.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30, egui 0.36, ffmpeg-next 9 (`codec`, `device`, `format`, `software-scaling`), serde.

**Spec:** `docs/superpowers/specs/2026-10-08-rasterwarp-video-inputs-design.md`

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- No new crates. `ffmpeg-next` gains exactly the `device` feature: `features = ["codec", "device", "format", "software-scaling"]`. Pinned versions stay: wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9.0.0, chrono 0.4, rfd 0.17, image 0.25, serde 1, serde_json 1.
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Running the tests needs the FFmpeg DLLs on `PATH`: `C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared\bin` (the same folder `.cargo/config.toml` names as `FFMPEG_DIR`). Without it a test binary exits with `STATUS_DLL_NOT_FOUND` (0xc0000135).
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Commit messages are plain UTF-8 without a byte-order mark.
- Exact values from the spec: memory for clips and camera buffers together `2 GB` (`2 << 30`); a frame ring at most `256` layers and `1 GB` (`1 << 30`) of video memory; camera buffer `1–30 s`, default `10 s`; camera retry every `3 s`; fallback frame rate `30 fps`; speed `−4…+4`; position `0–1`; delay `0–30 s`; slit depth `0–30 s` (held to what the ring fits when used). A camera that stops shows **"No Signal"**.
- When testing the app by hand, point `APPDATA` at a scratch folder so the developer's own settings and autosave are untouched.
- The camera test in `tests/video.rs` opens the first DirectShow camera and skips itself (with a message) when none is connected.
- Code in this plan was compiled, tested and run on the dev machine before the plan was written, and every task's end state was built and tested on its own. Transcribe it exactly. Each edit's "replace" text appears exactly once in the file when the edits are applied in the order given.

## Design decisions made while building (beyond the spec text)

1. **Virtual frame indices.** A clip's playhead counts on past the clip's ends (`clock × fps + offset`); `clip_frame` wraps (Loop), folds (Ping-pong) or clamps (Scrub) it to a stored frame. The frames around the playhead are then always a contiguous run, even across a loop point, so the GPU ring holds virtual index `k` in layer `k mod layers`. A camera's virtual index is its frame number. Because a virtual index means a different frame in another play mode, a mode change empties that view's ring (`FramesPass::forget`).
2. **The playhead's offset** (`Playback`) starts each new clip at its first frame, and leaving Scrub sets it so playback continues from the frame Scrub showed. The canvas and the preview each have one; when the preview starts showing something new it copies the canvas's, as its clocks do.
3. **The video clock always glides** during ramps (like the palette cycle): both sides advance at the lerped speed, so ramping speed is smooth and both sides stay equal.
4. **The preview copy is fitted from the canvas copy**, so it is never larger than it.
5. **Clip decoding is tested with a clip the capture encoder writes** (FFV1, frame `n` flat gray at level `10 n`), not `samples/anthony.mp4`, because the samples aren't in the repository. The memory error reads "`<file>` needs `<size>` of memory; shorten it or lower the canvas size"; other errors start "could not open `<file>`", in the app's lower-case style.
6. **Cameras** are listed with FFmpeg's `avdevice_list_input_sources` (video devices only, by friendly name) and opened as `video=<name>`. The capture thread is told to stop and left to exit on its own, so dropping a camera never blocks the window. Frame numbers keep counting across reconnections, and the buffer (with its memory) stays during No Signal so the last picture holds. A buffer that doesn't fit in memory is shortened, and the panel says "Buffer shortened to X s to fit in memory".
7. **A paused camera holds its picture**: its playhead stays at the arrival time where the pause began. Clips hold anyway, because the video clock stops.
8. **Projects** keep `source` and `background` and add `source_camera`, `background_camera`, `source_slit_map`, `background_slit_map` as top-level fields, through a flattened `InputFiles`; `Project::capture` takes `&InputFiles`. A camera's buffer length is clamped to 1–30 s when a project is read.
9. **Opening a project** shows the test card (source) or black (background) until its clip loads or its camera sends a picture. A missing video reads "Source video not found: …", a missing map "Background slit-scan map not found: …".
10. **The panel:** the Source and Background sections replace the old source line at the top and the Keying section's background buttons ("Open…" and "Test card" / "No background"). The delay and depth sliders show at most the buffer length and the ring's depth, and change the value only when edited, so drawing them never marks the project unsaved. Map without a map image works as Rows.
11. **`FullscreenPass::with_dimensions`** binds a 2D-array texture; `FullscreenPass::new` is unchanged for every existing pass.
12. **Verified in the real app:** `samples/flip.mp4` playing as the source (and restored from the autosave), slit-scan Rows on it, the Logi C270 listed and used as the keyed background, and a project with that camera (5 s buffer, 1 s delay, Columns slit-scan) opened with a clean header.

## File Structure

- `src/params.rs` — `VideoParams`, `PlayMode`, `Between`, `Slit`, `Role`, their ranges and clamping (Task 1).
- `src/blend.rs` — `Clocks.video`, `VideoFrame` in `FrameParams`, blending and gliding video settings (Task 1).
- `src/video/mod.rs` — `Pixels`, `Format`, `fit`, `Frame`, the shared memory `Budget`, and `Converter` (decoded frames to stored frames) (Tasks 2–4).
- `src/video/playhead.rs` — the pure playhead: `Playback`, `Sample`, `clip_sample`, `camera_sample`, `clip_frame`, ring layers and the depth cap (Task 2).
- `src/video/clip.rs` — decoding a clip into memory, and `Loading` on a worker thread (Task 3).
- `src/video/camera.rs` — listing cameras, `Camera` with its capture thread and rolling buffer (Task 4).
- `shaders/frames.wgsl`, `src/passes/frames.rs` — the frames pass and its texture-array ring (Task 5).
- `src/gpu.rs` — `FullscreenPass::with_dimensions` (Task 5).
- `src/passes/mod.rs`, `src/preview.rs` — renderer pictures from an image or a frames pass (Task 6).
- `src/session.rs` — `InputFiles`, `InputFile`, `CameraChoice` in projects (Task 7).
- `src/inputs.rs` — the app's inputs as they run: loading, cameras, feeding the renderers (Task 8).
- `src/app.rs` — wiring inputs into opening files and projects and each canvas frame (Tasks 7–9).
- `src/inputs_ui.rs`, `src/ui.rs` — the Source and Background panel sections (Task 9).
- `tests/video.rs` (new), `tests/smoke.rs` — clip, camera and GPU tests (Tasks 3–6).

---

### Task 1: Video playback settings and the video clock

Every bank, preset and cue gains playback settings for the source and the background. The video clock runs like the oscillator clocks, glides through ramps, and reaches the renderer in `FrameParams`.

**Files:**
- Modify: `src/params.rs`, `src/blend.rs`

**Interfaces:**
- Produces:
  - `params::{PlayMode { Loop, PingPong, Scrub }, Between { Nearest, Blend }, Slit { Off, Rows, Columns, Map }}`, each with `ALL` and `label()`, saved as `"loop"`/`"ping-pong"`/`"scrub"`, `"nearest"`/`"blend"`, `"off"`/`"rows"`/`"columns"`/`"map"`.
  - `params::VideoParams { mode, speed: f32, position: f32, between, delay: f32, slit, slit_depth: f32, slit_flip: bool }` (`Copy + serde`, default Loop, 1, 0, Blend, 0, Off, 1, false); `Params { …, source_video, background_video }`; `Params::video(Role) -> &VideoParams`, `Params::video_mut(Role) -> &mut VideoParams`.
  - `params::Role { Source, Background }` with `ALL`, `index() -> usize` (0, 1) and `label()`.
  - `ranges::{PLAY_SPEED, POSITION, DELAY, SLIT_DEPTH}`.
  - `blend::Clocks.video: [f64; 2]`; `blend::VideoFrame { mode, clock: f64, speed, position, between, delay, slit, slit_depth, slit_flip }`; `FrameParams.video: [VideoFrame; 2]`.

- [ ] **Step 1: Make the changes.**

In `src/blend.rs`, replace:

```rust
    Axis, ColorizeParams, Envelope, FeedbackParams, GlowParams, KeyParams, OSCILLATOR_COUNT,
    OscInput, OscSync, Oscillator, PALETTE_SIZE, Params, RasterParams, THRESHOLD_COUNT, Waveform,
    ranges, srgb_to_linear,
```

with:

```rust
    Axis, Between, ColorizeParams, Envelope, FeedbackParams, GlowParams, KeyParams,
    OSCILLATOR_COUNT, OscInput, OscSync, Oscillator, PALETTE_SIZE, Params, PlayMode, RasterParams,
    Role, Slit, THRESHOLD_COUNT, VideoParams, Waveform, ranges, srgb_to_linear,
```

In `src/blend.rs`, replace:

```rust
    pub cycle: f64,
}
```

with:

```rust
    pub cycle: f64,
    /// Clip time of each input's video, in seconds (unwrapped), indexed by
    /// [`Role::index`]. It runs in every play mode; Scrub just doesn't read it.
    pub video: [f64; 2],
}
```

In `src/blend.rs`, replace:

```rust
        self.cycle += f64::from(p.colorize.cycle_speed) * dt;
    }
```

with:

```rust
        self.cycle += f64::from(p.colorize.cycle_speed) * dt;
        for role in Role::ALL {
            self.video[role.index()] += f64::from(p.video(role).speed) * dt;
        }
    }
```

In `src/blend.rs`, replace:

```rust
/// oscillators read run at their own side's rates. The palette cycle always glides.
```

with:

```rust
/// oscillators read run at their own side's rates. The palette cycle and the video clocks
/// always glide.
```

In `src/blend.rs`, replace:

```rust
    to_clocks.cycle += cycle;
}
```

with:

```rust
    to_clocks.cycle += cycle;
    for role in Role::ALL {
        let (a, b) = (from.video(role), to.video(role));
        let step = f64::from(lerp_in(a.speed, b.speed, t, ranges::PLAY_SPEED)) * dt64;
        from_clocks.video[role.index()] += step;
        to_clocks.video[role.index()] += step;
    }
}
```

In `src/blend.rs`, replace:

```rust

/// Everything the renderer needs for one frame.
```

with:

```rust

/// One input's playback settings as this frame uses them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoFrame {
    pub mode: PlayMode,
    /// The video clock: clip time in seconds, unwrapped.
    pub clock: f64,
    /// Which way the clip is playing (its sign), for slit-scan.
    pub speed: f32,
    pub position: f32,
    pub between: Between,
    pub delay: f32,
    pub slit: Slit,
    pub slit_depth: f32,
    pub slit_flip: bool,
}

/// Everything the renderer needs for one frame.
```

In `src/blend.rs`, replace:

```rust
    pub key: KeyParams,
}
```

with:

```rust
    pub key: KeyParams,
    /// Indexed by [`Role::index`].
    pub video: [VideoFrame; 2],
}
```

In `src/blend.rs`, replace:

```rust
        key: if t >= 0.5 { to.key } else { from.key },
    }
```

with:

```rust
        key: if t >= 0.5 { to.key } else { from.key },
        video: Role::ALL.map(|role| {
            let i = role.index();
            let clock = lerp_phase(from_clocks.video[i], to_clocks.video[i], t);
            blend_video(from.video(role), to.video(role), t, clock)
        }),
    }
}

fn blend_video(a: &VideoParams, b: &VideoParams, t: f32, clock: f64) -> VideoFrame {
    let discrete = if t >= 0.5 { b } else { a };
    VideoFrame {
        mode: discrete.mode,
        clock,
        speed: lerp_in(a.speed, b.speed, t, ranges::PLAY_SPEED),
        position: lerp_in(a.position, b.position, t, ranges::POSITION),
        between: discrete.between,
        delay: lerp_in(a.delay, b.delay, t, ranges::DELAY),
        slit: discrete.slit,
        slit_depth: lerp_in(a.slit_depth, b.slit_depth, t, ranges::SLIT_DEPTH),
        slit_flip: discrete.slit_flip,
    }
```

In `src/blend.rs`, replace:

```rust
        assert_eq!(over.raster.beam_width, *ranges::BEAM_WIDTH.end());
    }
```

with:

```rust
        assert_eq!(over.raster.beam_width, *ranges::BEAM_WIDTH.end());
    }

    #[test]
    fn video_settings_lerp_and_switch_at_midpoint() {
        let a = Params::default();
        let mut b = a;
        let v = b.video_mut(Role::Background);
        v.mode = PlayMode::Scrub;
        v.speed = -3.0;
        v.position = 0.8;
        v.between = Between::Nearest;
        v.delay = 4.0;
        v.slit = Slit::Rows;
        v.slit_depth = 5.0;
        v.slit_flip = true;
        let early = blend(&a, &b, 0.25, Some(0.25), &rest(), &rest()).video[1];
        assert!((early.speed - 0.0).abs() < 1e-6);
        assert!((early.position - 0.2).abs() < 1e-6);
        assert!((early.delay - 1.0).abs() < 1e-6);
        assert!((early.slit_depth - 2.0).abs() < 1e-6);
        assert_eq!(early.mode, PlayMode::Loop);
        assert_eq!(
            (early.between, early.slit, early.slit_flip),
            (Between::Blend, Slit::Off, false)
        );
        let late = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest()).video[1];
        assert_eq!(late.mode, PlayMode::Scrub);
        assert_eq!(
            (late.between, late.slit, late.slit_flip),
            (Between::Nearest, Slit::Rows, true)
        );
        let source = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest()).video[0];
        assert_eq!(source.mode, PlayMode::Loop, "the source is unchanged");
    }

    #[test]
    fn video_clocks_run_at_their_speeds() {
        let mut p = Params::default();
        p.video_mut(Role::Source).speed = -2.0;
        let mut c = Clocks::default();
        c.advance(&p, 0.5);
        assert_eq!(c.video, [-1.0, 0.5]);
        let f = blend(&p, &p, 0.0, None, &c, &c);
        assert_eq!((f.video[0].clock, f.video[1].clock), (-1.0, 0.5));
    }

    #[test]
    fn ramps_glide_video_speed_on_both_sides() {
        let a = Params::default();
        let mut b = a;
        b.video_mut(Role::Source).speed = -3.0;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 0.5, 2.0);
        // Halfway from 1× to −3×: −1× for 2 s.
        assert!((from.video[0] + 2.0).abs() < 1e-9);
        assert_eq!(from, to, "both sides stay on one clock");
    }
```

In `src/params.rs`, replace:

```rust
    pub const SPEED_COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
}
```

with:

```rust
    pub const SPEED_COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
    /// Clip playback speed, × the clip's own frame rate; negative plays backwards.
    pub const PLAY_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const POSITION: RangeInclusive<f32> = 0.0..=1.0;
    /// Seconds. A camera's delay is also held to its buffer length when used.
    pub const DELAY: RangeInclusive<f32> = 0.0..=30.0;
    /// Seconds. The GPU's frame ring may allow less, which is applied when used.
    pub const SLIT_DEPTH: RangeInclusive<f32> = 0.0..=30.0;
}
```

In `src/params.rs`, replace:

```rust
    pub noise: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
```

with:

```rust
    pub noise: f32,
}

/// How a clip plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayMode {
    /// Plays at `speed` and wraps around at the ends.
    #[default]
    Loop,
    /// Plays at `speed` and bounces back at the ends.
    PingPong,
    /// Shows the frame at `position`; transitions and cues ramp through the clip.
    Scrub,
}

impl PlayMode {
    pub const ALL: [PlayMode; 3] = [PlayMode::Loop, PlayMode::PingPong, PlayMode::Scrub];

    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Loop => "Loop",
            PlayMode::PingPong => "Ping-pong",
            PlayMode::Scrub => "Scrub",
        }
    }
}

saved_names!(PlayMode {
    Loop => "loop",
    PingPong => "ping-pong",
    Scrub => "scrub",
});

/// What shows when the playhead sits between two frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Between {
    Nearest,
    #[default]
    Blend,
}

impl Between {
    pub const ALL: [Between; 2] = [Between::Nearest, Between::Blend];

    pub fn label(self) -> &'static str {
        match self {
            Between::Nearest => "Nearest",
            Between::Blend => "Blend",
        }
    }
}

saved_names!(Between {
    Nearest => "nearest",
    Blend => "blend",
});

/// Slit-scan: which pixels show older moments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slit {
    #[default]
    Off,
    /// The top row is now, the bottom row is the depth ago.
    Rows,
    /// The left column is now, the right column is the depth ago.
    Columns,
    /// A black-and-white map image: black is now, white is the depth ago.
    Map,
}

impl Slit {
    pub const ALL: [Slit; 4] = [Slit::Off, Slit::Rows, Slit::Columns, Slit::Map];

    pub fn label(self) -> &'static str {
        match self {
            Slit::Off => "Off",
            Slit::Rows => "Rows",
            Slit::Columns => "Columns",
            Slit::Map => "Map",
        }
    }
}

saved_names!(Slit {
    Off => "off",
    Rows => "rows",
    Columns => "columns",
    Map => "map",
});

/// How a video or camera input plays. Images ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoParams {
    /// Clips only.
    pub mode: PlayMode,
    /// Loop and Ping-pong: × the clip's own frame rate; negative plays backwards.
    pub speed: f32,
    /// Scrub: 0 = the first frame, 1 = the last.
    pub position: f32,
    pub between: Between,
    /// Cameras only: how many seconds behind live.
    pub delay: f32,
    pub slit: Slit,
    /// Seconds behind the playhead at the far end of the slit-scan map.
    pub slit_depth: f32,
    /// Reverses the slit-scan map, so its far end is now.
    pub slit_flip: bool,
}

impl Default for VideoParams {
    fn default() -> Self {
        Self {
            mode: PlayMode::Loop,
            speed: 1.0,
            position: 0.0,
            between: Between::Blend,
            delay: 0.0,
            slit: Slit::Off,
            slit_depth: 1.0,
            slit_flip: false,
        }
    }
}

/// The two inputs: the source the passes manipulate, and the background keyed levels
/// show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Source,
    Background,
}

impl Role {
    pub const ALL: [Role; 2] = [Role::Source, Role::Background];

    pub fn index(self) -> usize {
        match self {
            Role::Source => 0,
            Role::Background => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Source => "Source",
            Role::Background => "Background",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
```

In `src/params.rs`, replace:

```rust
    pub key: KeyParams,
}
```

with:

```rust
    pub key: KeyParams,
    pub source_video: VideoParams,
    pub background_video: VideoParams,
}
```

In `src/params.rs`, replace:

```rust
            },
        }
```

with:

```rust
            },
            source_video: VideoParams::default(),
            background_video: VideoParams::default(),
        }
```

In `src/params.rs`, replace:

```rust
        self
```

with:

```rust

        for v in [&mut self.source_video, &mut self.background_video] {
            clamp(&mut v.speed, ranges::PLAY_SPEED);
            clamp(&mut v.position, ranges::POSITION);
            clamp(&mut v.delay, ranges::DELAY);
            clamp(&mut v.slit_depth, ranges::SLIT_DEPTH);
        }
        self
    }

    pub fn video(&self, role: Role) -> &VideoParams {
        match role {
            Role::Source => &self.source_video,
            Role::Background => &self.background_video,
        }
    }

    pub fn video_mut(&mut self, role: Role) -> &mut VideoParams {
        match role {
            Role::Source => &mut self.source_video,
            Role::Background => &mut self.background_video,
        }
```

In `src/params.rs`, replace:

```rust
        assert!(ranges::SPEED_COMPENSATION.contains(&p.raster.speed_compensation));
    }
```

with:

```rust
        assert!(ranges::SPEED_COMPENSATION.contains(&p.raster.speed_compensation));
        for v in [p.source_video, p.background_video] {
            assert!(ranges::PLAY_SPEED.contains(&v.speed));
            assert!(ranges::POSITION.contains(&v.position));
            assert!(ranges::DELAY.contains(&v.delay));
            assert!(ranges::SLIT_DEPTH.contains(&v.slit_depth));
        }
    }

    #[test]
    fn video_settings_default_to_looping_at_normal_speed() {
        let v = VideoParams::default();
        assert_eq!((v.mode, v.speed, v.position), (PlayMode::Loop, 1.0, 0.0));
        assert_eq!((v.between, v.delay), (Between::Blend, 0.0));
        assert_eq!((v.slit, v.slit_depth, v.slit_flip), (Slit::Off, 1.0, false));
        let p = Params::default();
        assert_eq!(*p.video(Role::Source), v);
        assert_eq!(*p.video(Role::Background), v);
    }

    #[test]
    fn video_settings_are_clamped() {
        let mut p = Params::default();
        let v = p.video_mut(Role::Background);
        v.speed = -9.0;
        v.position = 1.5;
        v.delay = 99.0;
        v.slit_depth = -1.0;
        let v = p.clamped().background_video;
        assert_eq!((v.speed, v.position), (-4.0, 1.0));
        assert_eq!((v.delay, v.slit_depth), (30.0, 0.0));
    }
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --lib video`. Expected: the new tests in `params` and `blend` pass, among them `video_settings_lerp_and_switch_at_midpoint`, `video_clocks_run_at_their_speeds` and `ramps_glide_video_speed_on_both_sides`.

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `196 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `17 passed` in `tests/smoke.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add src/params.rs src/blend.rs
git commit -m "feat: video playback settings in the banks, with a video clock" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 2: The playhead

Pure maths, with no GPU or files: where each input's playhead is, which frames any pixel may show, and how many ring layers the GPU may use.

**Files:**
- Create: `src/video/mod.rs`, `src/video/playhead.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `VideoFrame`, `PlayMode`, `Between`, `Slit` (Task 1).
- Produces (in `video::playhead`):
  - `Sample { base: f64, reach: f64, first: i64, last: i64, between, slit, flip: bool }` and `Sample::window() -> RangeInclusive<i64>`.
  - `Playback` (`Copy + Default`) with `index(&mut self, v: &VideoFrame, count: u32, fps: f64) -> f64`.
  - `scrub_index(position: f32, count: u32) -> f64`; `clip_frame(mode: PlayMode, k: i64, count: u32) -> u32`; `layer(k: i64, layers: u32) -> u32`.
  - `max_layers(device_layers: u32, layer_bytes: u64) -> u32`; `max_depth(layers: u32, fps: f64) -> f32`; `MAX_LAYERS = 256`, `VRAM_BUDGET = 1 << 30`, `FALLBACK_FPS = 30.0`.
  - `clip_sample(v: &VideoFrame, index: f64, count: u32, fps: f64, layers: u32) -> Sample`.
  - `Arrival { seq: i64, time: f64 }`; `index_at(&[Arrival], time: f64) -> f64`; `camera_fps(&[Arrival]) -> f64`; `camera_sample(v: &VideoFrame, &[Arrival], layers: u32) -> Option<Sample>`.

- [ ] **Step 1: Make the changes.**

In `src/lib.rs`, replace:

```rust
pub mod ui;
```

with:

```rust
pub mod ui;
pub mod video;
```

Create `src/video/mod.rs`:

```rust
//! Video inputs: clips decoded into memory and live cameras, and the playhead that
//! picks their frames.

pub mod playhead;
```

Create `src/video/playhead.rs`:

```rust
//! Where an input's playhead is: from the playback settings and the video clock to the
//! frames to show and how to mix them. Pure maths, with no GPU or files.
//!
//! Frames are addressed by *virtual index*: consecutive whole numbers along the way the
//! playhead travels. A clip's virtual index keeps counting past its ends (the clip frame
//! is found by wrapping or folding it), so the frames around the playhead are always a
//! contiguous run, even across a loop point. A camera's virtual index is its frame
//! number.

use crate::blend::VideoFrame;
use crate::params::{Between, PlayMode, Slit};

/// The most texture-array layers a frame ring uses (wgpu's default
/// `max_texture_array_layers`).
pub const MAX_LAYERS: u32 = 256;

/// Video memory one frame ring may use.
pub const VRAM_BUDGET: u64 = 1 << 30;

/// A camera's frame rate when it can't be measured yet.
pub const FALLBACK_FPS: f64 = 30.0;

/// What the frames pass draws for one input this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The playhead, as a fractional virtual index.
    pub base: f64,
    /// How many frames behind `base` the far end of the slit-scan map lies; negative
    /// when a clip plays backwards (behind is then ahead). 0 without slit-scan.
    pub reach: f64,
    /// The virtual indices any pixel may show, which must be on the GPU.
    pub first: i64,
    pub last: i64,
    pub between: Between,
    /// Off when there's no slit-scan this frame.
    pub slit: Slit,
    pub flip: bool,
}

impl Sample {
    /// The virtual indices to have on the GPU, oldest first.
    pub fn window(&self) -> std::ops::RangeInclusive<i64> {
        self.first..=self.last
    }
}

/// A clip's playback state between frames: where its virtual index starts, and the frame
/// Scrub showed last.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playback {
    /// Added to clock × fps. None until the first frame, which then starts the clip at
    /// its first frame.
    offset: Option<f64>,
    /// The index Scrub showed on the previous frame, if it was in Scrub.
    scrubbed: Option<f64>,
}

impl Playback {
    /// The playhead's fractional virtual index for a clip of `count` frames at `fps`.
    /// Leaving Scrub carries on from the frame Scrub showed.
    pub fn index(&mut self, v: &VideoFrame, count: u32, fps: f64) -> f64 {
        if v.mode == PlayMode::Scrub {
            let index = scrub_index(v.position, count);
            self.scrubbed = Some(index);
            return index;
        }
        let playing = v.clock * fps;
        if let Some(shown) = self.scrubbed.take() {
            self.offset = Some(shown - playing);
        }
        playing + *self.offset.get_or_insert(-playing)
    }
}

/// Scrub's index for `position` (0–1) in a clip of `count` frames.
pub fn scrub_index(position: f32, count: u32) -> f64 {
    f64::from(position.clamp(0.0, 1.0)) * f64::from(count.saturating_sub(1))
}

/// The clip frame (0..count) that virtual index `k` shows.
pub fn clip_frame(mode: PlayMode, k: i64, count: u32) -> u32 {
    let count = i64::from(count.max(1));
    let frame = match mode {
        PlayMode::Loop => k.rem_euclid(count),
        PlayMode::PingPong if count == 1 => 0,
        PlayMode::PingPong => {
            // Forwards then backwards, without showing the end frames twice.
            let period = 2 * (count - 1);
            let m = k.rem_euclid(period);
            if m < count { m } else { period - m }
        }
        PlayMode::Scrub => k.clamp(0, count - 1),
    };
    frame as u32
}

/// The ring layer that holds virtual index `k`.
pub fn layer(k: i64, layers: u32) -> u32 {
    k.rem_euclid(i64::from(layers.max(1))) as u32
}

/// How many layers a frame ring may have, for frames of `layer_bytes` each on a device
/// allowing `device_layers`. At least 2, so the playhead can always blend.
pub fn max_layers(device_layers: u32, layer_bytes: u64) -> u32 {
    let budget = (VRAM_BUDGET / layer_bytes.max(1)).min(u64::from(u32::MAX)) as u32;
    MAX_LAYERS.min(device_layers).min(budget).max(2)
}

/// The deepest slit-scan, in seconds, a ring of `layers` layers holds at `fps`.
pub fn max_depth(layers: u32, fps: f64) -> f32 {
    (f64::from(layers.saturating_sub(2)) / fps.max(1e-6)) as f32
}

/// Slit-scan's reach in frames, held so the window fits in `layers` layers. 0 without
/// slit-scan.
fn reach_frames(v: &VideoFrame, fps: f64, layers: u32) -> f64 {
    if v.slit == Slit::Off {
        return 0.0;
    }
    (f64::from(v.slit_depth.max(0.0)) * fps).min(f64::from(layers.saturating_sub(2)))
}

/// What a clip shows this frame. `index` is the playhead from [`Playback::index`];
/// `layers` is the most layers its ring may have.
pub fn clip_sample(v: &VideoFrame, index: f64, count: u32, fps: f64, layers: u32) -> Sample {
    let depth = reach_frames(v, fps, layers);
    let (reach, first, last) = if v.mode == PlayMode::Scrub {
        // Behind is towards the first frame, which holds.
        let last_frame = i64::from(count.saturating_sub(1));
        let first = ((index - depth).floor() as i64).max(0);
        let last = (index.floor() as i64 + 1).min(last_frame);
        (depth, first, last.max(first))
    } else {
        // Behind is the way the playhead came.
        let reach = if v.speed < 0.0 { -depth } else { depth };
        let (lo, hi) = (index.min(index - reach), index.max(index - reach));
        (reach, lo.floor() as i64, hi.floor() as i64 + 1)
    };
    Sample {
        base: index,
        reach,
        first,
        last,
        between: v.between,
        slit: if depth > 0.0 { v.slit } else { Slit::Off },
        flip: v.slit_flip,
    }
}

/// When a camera frame arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrival {
    /// The camera's frame number (its virtual index).
    pub seq: i64,
    /// Seconds, on any clock that only moves forwards.
    pub time: f64,
}

/// The fractional frame number on show at `time`, between the frames that arrived
/// either side of it. Clamped to the frames there are. `arrivals` is oldest first and
/// not empty.
pub fn index_at(arrivals: &[Arrival], time: f64) -> f64 {
    let after = arrivals.partition_point(|a| a.time <= time);
    if after == 0 {
        return arrivals[0].seq as f64;
    }
    if after == arrivals.len() {
        return arrivals[after - 1].seq as f64;
    }
    let (a, b) = (arrivals[after - 1], arrivals[after]);
    let span = b.time - a.time;
    let fraction = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };
    a.seq as f64 + fraction * (b.seq - a.seq) as f64
}

/// The camera's measured frame rate over the buffer.
pub fn camera_fps(arrivals: &[Arrival]) -> f64 {
    match (arrivals.first(), arrivals.last()) {
        (Some(a), Some(b)) if b.time > a.time => (b.seq - a.seq) as f64 / (b.time - a.time),
        _ => FALLBACK_FPS,
    }
}

/// What a camera shows this frame: live minus the delay, with slit-scan reaching further
/// back. Nothing before its first frame arrives.
pub fn camera_sample(v: &VideoFrame, arrivals: &[Arrival], layers: u32) -> Option<Sample> {
    let (oldest, newest) = (arrivals.first()?, arrivals.last()?);
    let base = index_at(arrivals, newest.time - f64::from(v.delay.max(0.0)));
    let depth = reach_frames(v, camera_fps(arrivals), layers);
    let first = ((base - depth).floor() as i64).max(oldest.seq);
    let last = (base.floor() as i64 + 1).min(newest.seq).max(first);
    Some(Sample {
        base,
        reach: depth,
        first,
        last,
        between: v.between,
        slit: if depth > 0.0 { v.slit } else { Slit::Off },
        flip: v.slit_flip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    fn video(edit: impl FnOnce(&mut VideoFrame)) -> VideoFrame {
        let mut v = FrameParams::at_rest(&Params::default()).video[0];
        edit(&mut v);
        v
    }

    #[test]
    fn loop_wraps_both_ways() {
        assert_eq!(clip_frame(PlayMode::Loop, 9, 10), 9);
        assert_eq!(clip_frame(PlayMode::Loop, 10, 10), 0);
        assert_eq!(clip_frame(PlayMode::Loop, 23, 10), 3);
        assert_eq!(clip_frame(PlayMode::Loop, -1, 10), 9);
        assert_eq!(clip_frame(PlayMode::Loop, -11, 10), 9);
    }

    #[test]
    fn ping_pong_folds_without_doubling_the_ends() {
        let frames: Vec<u32> = (0..10)
            .map(|k| clip_frame(PlayMode::PingPong, k, 4))
            .collect();
        assert_eq!(frames, [0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
        assert_eq!(clip_frame(PlayMode::PingPong, -1, 4), 1);
        assert_eq!(clip_frame(PlayMode::PingPong, 7, 1), 0, "a one-frame clip");
    }

    #[test]
    fn scrub_maps_position_onto_the_clip() {
        assert_eq!(scrub_index(0.0, 61), 0.0);
        assert_eq!(scrub_index(0.5, 61), 30.0);
        assert_eq!(scrub_index(1.0, 61), 60.0);
        assert_eq!(clip_frame(PlayMode::Scrub, 99, 61), 60);
        assert_eq!(clip_frame(PlayMode::Scrub, -3, 61), 0);
    }

    #[test]
    fn a_new_clip_starts_at_its_first_frame_and_follows_the_clock() {
        let mut playback = Playback::default();
        let at = |clock| video(|v| v.clock = clock);
        assert_eq!(playback.index(&at(7.5), 48, 24.0), 0.0);
        assert_eq!(playback.index(&at(8.0), 48, 24.0), 12.0);
        // A clock running backwards (negative speed) counts down.
        assert_eq!(playback.index(&at(7.0), 48, 24.0), -12.0);
    }

    #[test]
    fn leaving_scrub_carries_on_from_the_frame_on_screen() {
        let mut playback = Playback::default();
        let scrub = video(|v| {
            v.mode = PlayMode::Scrub;
            v.position = 0.5;
            v.clock = 3.0;
        });
        assert_eq!(playback.index(&scrub, 21, 10.0), 10.0);
        let looping = |clock| video(|v| v.clock = clock);
        assert_eq!(playback.index(&looping(3.1), 21, 10.0), 10.0);
        assert!((playback.index(&looping(3.5), 21, 10.0) - 14.0).abs() < 1e-9);
    }

    #[test]
    fn nearest_and_blend_come_through() {
        let v = video(|v| v.between = Between::Nearest);
        let s = clip_sample(&v, 4.25, 10, 24.0, 256);
        assert_eq!((s.base, s.reach, s.between), (4.25, 0.0, Between::Nearest));
        assert_eq!(s.window(), 4..=5, "the two frames either side");
        assert_eq!(s.slit, Slit::Off);
        let s = clip_sample(&video(|_| {}), 4.25, 10, 24.0, 256);
        assert_eq!(s.between, Between::Blend);
    }

    #[test]
    fn slit_scan_reaches_back_the_way_the_clip_came() {
        let forward = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 0.5;
        });
        let s = clip_sample(&forward, 30.25, 100, 24.0, 256);
        assert_eq!((s.reach, s.slit), (12.0, Slit::Rows));
        assert_eq!(s.window(), 18..=31);
        let backward = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 0.5;
            v.speed = -1.0;
        });
        let s = clip_sample(&backward, 30.25, 100, 24.0, 256);
        assert_eq!(s.reach, -12.0);
        assert_eq!(s.window(), 30..=43);
    }

    #[test]
    fn scrub_slit_scan_holds_at_the_first_frame() {
        let v = video(|v| {
            v.mode = PlayMode::Scrub;
            v.slit = Slit::Columns;
            v.slit_depth = 1.0;
        });
        let s = clip_sample(&v, 5.0, 10, 24.0, 256);
        assert_eq!(s.window(), 0..=6);
        let s = clip_sample(&v, 9.0, 10, 24.0, 256);
        assert_eq!(s.window(), 0..=9, "never past the last frame");
    }

    #[test]
    fn slit_scan_depth_is_capped_by_the_ring() {
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 30.0;
        });
        let s = clip_sample(&v, 1000.5, 5000, 30.0, 64);
        assert_eq!(s.reach, 62.0);
        let size = s.last - s.first + 1;
        assert!(size <= 64, "{size} frames");
        assert_eq!(max_depth(64, 30.0), 62.0 / 30.0);
    }

    #[test]
    fn zero_depth_is_no_slit_scan() {
        let v = video(|v| {
            v.slit = Slit::Map;
            v.slit_depth = 0.0;
        });
        assert_eq!(clip_sample(&v, 3.0, 10, 24.0, 256).slit, Slit::Off);
    }

    #[test]
    fn ring_layers_wrap_and_respect_the_budgets() {
        assert_eq!(layer(5, 4), 1);
        assert_eq!(layer(-1, 4), 3);
        assert_eq!(max_layers(2048, 1280 * 720), 256);
        assert_eq!(max_layers(100, 1280 * 720), 100);
        // 1080p RGBA: about 8.3 MB a layer, so 1 GB holds 129.
        assert_eq!(max_layers(2048, 1920 * 1080 * 4), 129);
        assert_eq!(max_layers(2048, u64::MAX), 2);
    }

    fn arrivals(times: &[f64]) -> Vec<Arrival> {
        times
            .iter()
            .enumerate()
            .map(|(i, &time)| Arrival {
                seq: 100 + i as i64,
                time,
            })
            .collect()
    }

    #[test]
    fn camera_frames_are_found_by_arrival_time() {
        let a = arrivals(&[10.0, 10.1, 10.2, 10.4]);
        assert_eq!(index_at(&a, 10.2), 102.0);
        assert!((index_at(&a, 10.3) - 102.5).abs() < 1e-9);
        assert_eq!(index_at(&a, 9.0), 100.0, "older than the buffer");
        assert_eq!(index_at(&a, 11.0), 103.0);
        assert!((camera_fps(&a) - 7.5).abs() < 1e-9);
        assert_eq!(camera_fps(&a[..1]), FALLBACK_FPS);
    }

    #[test]
    fn camera_delay_and_slit_scan_stay_in_the_buffer() {
        let times: Vec<f64> = (0..31).map(|i| i as f64 / 30.0).collect();
        let a = arrivals(&times);
        let live = camera_sample(&video(|_| {}), &a, 256).unwrap();
        assert_eq!(live.base, 130.0);
        assert_eq!(live.window(), 130..=130);
        let delayed = camera_sample(&video(|v| v.delay = 0.5), &a, 256).unwrap();
        assert!((delayed.base - 115.0).abs() < 1e-6);
        let deep = camera_sample(
            &video(|v| {
                v.delay = 0.9;
                v.slit = Slit::Rows;
                v.slit_depth = 1.0;
            }),
            &a,
            256,
        )
        .unwrap();
        assert!((deep.reach - 30.0).abs() < 1e-6);
        assert_eq!(deep.first, 100, "clamped to the oldest frame");
        assert!(camera_sample(&video(|_| {}), &[], 256).is_none());
    }
}
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --lib playhead`. Expected: `13 passed`.

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `209 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `17 passed` in `tests/smoke.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/video
git commit -m "feat: the playhead: play modes, slit-scan reach and frame rings" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 3: Clips decoded into memory

The frame store's types and the shared memory budget, and decoding a video file's first stream into luma or RGBA frames (plus preview copies) on a worker thread.

**Files:**
- Create: `src/video/clip.rs`, `tests/video.rs`
- Modify: `src/video/mod.rs`

**Interfaces:**
- Consumes: `Role` (Task 1), `preview::size_for`.
- Produces (in `video`):
  - `MEMORY_LIMIT: u64 = 2 << 30`; `Pixels { Luma, Rgba }` with `for_role(Role)`, `bytes() -> u32`, `ffmpeg() -> ff::format::Pixel`.
  - `Format { pixels, size: (u32, u32), small: (u32, u32) }` with `Format::new(pixels, picture, canvas)`, `bytes_at(size) -> u64`, `frame_bytes() -> u64`; `fit(picture, bounds) -> (u32, u32)`.
  - `Frame { full: Vec<u8>, small: Vec<u8> }`; `Budget` (`Clone`, shared) with `new(limit)`, `available()`, `take(bytes) -> Option<Reservation>`; `Reservation` with `grow(bytes) -> bool`, `bytes()`, released on drop; `describe_bytes(u64) -> String`.
  - `Converter::new(Format)`, `Converter::convert(&ff::frame::Video) -> Result<Frame>`.
  - `clip::{Clip { format, fps: f64, frames: Vec<Frame>, .. }, Clip::count() -> u32, load(path, pixels, canvas, &Budget, &AtomicU32, &AtomicBool) -> Result<Clip>, Loading}`; `Loading::start(path, pixels, canvas, &Budget)`, `progress() -> f32`, `poll() -> Option<Result<Clip>>`, `wait() -> Result<Clip>`, `path: PathBuf`; dropping a `Loading` cancels it.

- [ ] **Step 1: Make the changes.**

Create `src/video/clip.rs`:

```rust
//! Clips: a video file decoded whole into memory on a worker thread, so every frame is
//! there at once for playing backwards, scrubbing and slit-scan.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;

use super::{Budget, Converter, Format, Frame, Pixels, Reservation, describe_bytes};

/// A clip's frame rate when the file doesn't give one.
pub const FALLBACK_FPS: f64 = 30.0;

/// A decoded clip.
#[derive(Debug)]
pub struct Clip {
    pub format: Format,
    /// The stream's average frame rate.
    pub fps: f64,
    pub frames: Vec<Frame>,
    /// The memory the frames take from the budget, given back when the clip goes.
    _memory: Reservation,
}

impl Clip {
    pub fn count(&self) -> u32 {
        self.frames.len() as u32
    }
}

/// The file name, for messages.
fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Decodes the first video stream of `path` into frames for a `canvas`-sized canvas.
/// `progress` counts up to 1000 as it goes; setting `cancel` stops it.
pub fn load(
    path: &Path,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Clip> {
    let name = name(path);
    decode(path, &name, pixels, canvas, budget, progress, cancel)
        .with_context(|| format!("could not open {name}"))
}

#[allow(clippy::too_many_arguments)]
fn decode(
    path: &Path,
    name: &str,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Clip> {
    ff::init().context("could not initialise FFmpeg")?;
    let mut input = ff::format::input(path)?;
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let index = stream.index();
    let rate = stream.avg_frame_rate();
    let fps = if rate.numerator() > 0 && rate.denominator() > 0 {
        f64::from(rate)
    } else {
        FALLBACK_FPS
    };
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    let format = Format::new(pixels, (decoder.width(), decoder.height()), canvas);
    // The frame count the file claims, or one from its length, for progress and the
    // memory check.
    let expected = match stream.frames() {
        n if n > 0 => n as u64,
        _ => {
            let seconds = input.duration().max(0) as f64 / f64::from(ff::ffi::AV_TIME_BASE);
            (seconds * fps).ceil() as u64
        }
    };
    let too_big = |frames: u64| {
        anyhow!(
            "{name} needs {} of memory; shorten it or lower the canvas size",
            describe_bytes(frames * format.frame_bytes())
        )
    };
    let mut memory = budget
        .take(expected * format.frame_bytes())
        .ok_or_else(|| too_big(expected))?;
    let mut converter = Converter::new(format);
    let mut frames = Vec::with_capacity(expected as usize);
    let mut decoded = ff::frame::Video::empty();
    let mut receive = |decoder: &mut ff::decoder::Video, frames: &mut Vec<Frame>| -> Result<()> {
        while decoder.receive_frame(&mut decoded).is_ok() {
            if frames.len() as u64 >= expected && !memory.grow(format.frame_bytes()) {
                return Err(too_big(frames.len() as u64 + 1));
            }
            frames.push(converter.convert(&decoded)?);
            let done = frames.len() as u64 * 1000 / expected.max(1);
            progress.store(done.min(1000) as u32, Ordering::Relaxed);
        }
        Ok(())
    };
    for (stream, packet) in input.packets() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if stream.index() == index {
            decoder.send_packet(&packet)?;
            receive(&mut decoder, &mut frames)?;
        }
    }
    decoder.send_eof()?;
    receive(&mut decoder, &mut frames)?;
    if frames.is_empty() {
        bail!("no frames");
    }
    // Give back what the file over-claimed.
    let used = frames.len() as u64 * format.frame_bytes();
    let memory = if memory.bytes() > used {
        drop(memory);
        budget
            .take(used)
            .ok_or_else(|| too_big(frames.len() as u64))?
    } else {
        memory
    };
    progress.store(1000, Ordering::Relaxed);
    Ok(Clip {
        format,
        fps,
        frames,
        _memory: memory,
    })
}

/// A clip being decoded on a worker thread. Dropping it cancels the decode.
pub struct Loading {
    pub path: PathBuf,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Clip>>,
}

impl Loading {
    pub fn start(path: &Path, pixels: Pixels, canvas: (u32, u32), budget: &Budget) -> Self {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, result) = mpsc::channel();
        let (owned, budget) = (path.to_path_buf(), budget.clone());
        let (p, c) = (progress.clone(), cancel.clone());
        let spawned = std::thread::Builder::new()
            .name("clip loader".into())
            .spawn(move || {
                let _ = send.send(load(&owned, pixels, canvas, &budget, &p, &c));
            });
        if let Err(err) = spawned {
            let (send, failed) = mpsc::channel();
            let _ = send.send(Err(anyhow!(
                "could not start loading {}: {err}",
                name(path)
            )));
            return Self {
                path: path.to_path_buf(),
                progress,
                cancel,
                result: failed,
            };
        }
        Self {
            path: path.to_path_buf(),
            progress,
            cancel,
            result,
        }
    }

    /// How far it got, 0–1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The clip, or why it failed, once the worker is done.
    pub fn poll(&self) -> Option<Result<Clip>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "the clip loader for {} stopped",
                name(&self.path)
            ))),
        }
    }

    /// Waits for the worker to finish.
    pub fn wait(&self) -> Result<Clip> {
        self.result
            .recv()
            .map_err(|_| anyhow!("the clip loader for {} stopped", name(&self.path)))?
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
```

In `src/video/mod.rs`, replace:

```rust
pub mod playhead;
```

with:

```rust
pub mod clip;
pub mod playhead;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ffmpeg_next as ff;

use crate::params::Role;
use crate::preview;

/// How much memory clips and camera buffers may hold together.
pub const MEMORY_LIMIT: u64 = 2 << 30;

/// How an input's frames are stored: grayscale for the source, color for the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pixels {
    Luma,
    Rgba,
}

impl Pixels {
    pub fn for_role(role: Role) -> Self {
        match role {
            Role::Source => Pixels::Luma,
            Role::Background => Pixels::Rgba,
        }
    }

    pub fn bytes(self) -> u32 {
        match self {
            Pixels::Luma => 1,
            Pixels::Rgba => 4,
        }
    }

    pub fn ffmpeg(self) -> ff::format::Pixel {
        match self {
            Pixels::Luma => ff::format::Pixel::GRAY8,
            Pixels::Rgba => ff::format::Pixel::RGBA,
        }
    }
}

/// The sizes an input's frames are stored at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub pixels: Pixels,
    /// For the canvas: the picture fitted inside the canvas, never scaled up.
    pub size: (u32, u32),
    /// For the off-air preview: that fitted inside the preview the same way.
    pub small: (u32, u32),
}

/// `picture` scaled to fit inside `bounds`, keeping its aspect ratio, never scaled up,
/// and at least 1×1.
pub fn fit(picture: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (w, h) = (f64::from(picture.0.max(1)), f64::from(picture.1.max(1)));
    let scale = (f64::from(bounds.0) / w)
        .min(f64::from(bounds.1) / h)
        .min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

impl Format {
    /// Frames of a `picture`-sized video for a canvas of `canvas` pixels.
    pub fn new(pixels: Pixels, picture: (u32, u32), canvas: (u32, u32)) -> Self {
        let size = fit(picture, canvas);
        Self {
            pixels,
            size,
            // Never bigger than the canvas copy.
            small: fit(size, preview::size_for(canvas)),
        }
    }

    /// Bytes in one row-major, tightly packed frame of `size`.
    pub fn bytes_at(&self, size: (u32, u32)) -> u64 {
        u64::from(size.0) * u64::from(size.1) * u64::from(self.pixels.bytes())
    }

    /// Bytes one stored frame (both copies) takes.
    pub fn frame_bytes(&self) -> u64 {
        self.bytes_at(self.size) + self.bytes_at(self.small)
    }
}

/// One stored frame, tightly packed rows in its [`Format`].
#[derive(Debug, PartialEq)]
pub struct Frame {
    pub full: Vec<u8>,
    pub small: Vec<u8>,
}

/// The memory clips and camera buffers share. Clones share the same count.
#[derive(Clone, Debug)]
pub struct Budget {
    used: Arc<AtomicU64>,
    limit: u64,
}

impl Budget {
    pub fn new(limit: u64) -> Self {
        Self {
            used: Arc::new(AtomicU64::new(0)),
            limit,
        }
    }

    /// Bytes not yet taken.
    pub fn available(&self) -> u64 {
        self.limit.saturating_sub(self.used.load(Ordering::Acquire))
    }

    /// Takes `bytes`, or nothing if they don't fit. They're given back when the
    /// reservation is dropped.
    pub fn take(&self, bytes: u64) -> Option<Reservation> {
        let mut reservation = Reservation {
            budget: self.clone(),
            bytes: 0,
        };
        reservation.grow(bytes).then_some(reservation)
    }
}

/// Memory taken from a [`Budget`].
#[derive(Debug)]
pub struct Reservation {
    budget: Budget,
    bytes: u64,
}

impl Reservation {
    /// Takes `bytes` more, or nothing if they don't fit.
    pub fn grow(&mut self, bytes: u64) -> bool {
        let limit = self.budget.limit;
        let grown = self
            .budget
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|&total| total <= limit)
            });
        if grown.is_ok() {
            self.bytes += bytes;
        }
        grown.is_ok()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// `bytes` for people: "3.1 GB" or "240 MB".
pub fn describe_bytes(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    const MB: f64 = (1u64 << 20) as f64;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else {
        format!("{:.0} MB", (bytes / MB).ceil())
    }
}

/// Converts decoded video frames to stored [`Frame`]s: both copies, tightly packed.
/// Scalers are made for the first frame and remade if the decoded picture changes.
pub struct Converter {
    format: Format,
    scalers: Option<(
        ScalerKey,
        ff::software::scaling::Context,
        ff::software::scaling::Context,
    )>,
}

type ScalerKey = (ff::format::Pixel, u32, u32);

impl Converter {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            scalers: None,
        }
    }

    pub fn convert(&mut self, decoded: &ff::frame::Video) -> anyhow::Result<Frame> {
        let key = (decoded.format(), decoded.width(), decoded.height());
        if self.scalers.as_ref().is_none_or(|(k, ..)| *k != key) {
            let make = |size: (u32, u32)| {
                ff::software::scaling::Context::get(
                    key.0,
                    key.1,
                    key.2,
                    self.format.pixels.ffmpeg(),
                    size.0,
                    size.1,
                    ff::software::scaling::Flags::AREA,
                )
            };
            self.scalers = Some((key, make(self.format.size)?, make(self.format.small)?));
        }
        let (_, full, small) = self.scalers.as_mut().expect("scalers were just made");
        let bytes = self.format.pixels.bytes() as usize;
        Ok(Frame {
            full: scale(full, decoded, bytes)?,
            small: scale(small, decoded, bytes)?,
        })
    }
}

/// Runs `scaler` on `decoded` and returns its rows tightly packed.
fn scale(
    scaler: &mut ff::software::scaling::Context,
    decoded: &ff::frame::Video,
    bytes_per_pixel: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut out = ff::frame::Video::empty();
    scaler.run(decoded, &mut out)?;
    let row = out.width() as usize * bytes_per_pixel;
    let stride = out.stride(0);
    let data = out.data(0);
    let mut packed = Vec::with_capacity(row * out.height() as usize);
    for y in 0..out.height() as usize {
        packed.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_fit_inside_without_growing() {
        assert_eq!(fit((1280, 736), (1920, 1080)), (1280, 736));
        assert_eq!(fit((3840, 2160), (1920, 1080)), (1920, 1080));
        assert_eq!(fit((1280, 736), (480, 270)), (470, 270));
        assert_eq!(fit((640, 480), (1920, 1080)), (640, 480));
        assert_eq!(fit((10_000, 1), (100, 100)), (100, 1));
    }

    #[test]
    fn formats_store_a_canvas_copy_and_a_preview_copy() {
        let f = Format::new(Pixels::Rgba, (3840, 2160), (1920, 1080));
        assert_eq!((f.size, f.small), ((1920, 1080), (480, 270)));
        assert_eq!(f.frame_bytes(), (1920 * 1080 + 480 * 270) * 4);
        assert_eq!(Pixels::for_role(Role::Source), Pixels::Luma);
        assert_eq!(Pixels::for_role(Role::Background), Pixels::Rgba);
    }

    #[test]
    fn the_budget_is_shared_and_given_back() {
        let budget = Budget::new(100);
        let mut a = budget.take(60).unwrap();
        assert!(budget.clone().take(50).is_none(), "clones share the count");
        assert!(!a.grow(41));
        assert!(a.grow(40));
        assert_eq!((a.bytes(), budget.available()), (100, 0));
        drop(a);
        assert_eq!(budget.available(), 100);
    }

    #[test]
    fn sizes_read_in_megabytes_or_gigabytes() {
        assert_eq!(describe_bytes(240 << 20), "240 MB");
        assert_eq!(describe_bytes(3 << 30 | 1 << 27), "3.1 GB");
    }
}
```

Create `tests/video.rs`:

```rust
//! Video inputs: clips written with the capture encoder decode back into the frames they
//! were made from; cameras deliver frames when one is connected.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32};

use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};
use rasterwarp::rate::FrameRate;
use rasterwarp::video::clip::{self, Loading};
use rasterwarp::video::{Budget, MEMORY_LIMIT, Pixels};

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 24;

/// A fresh path in a per-test temporary directory.
fn temp_file(test: &str, file: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(file)
}

/// Frame `n` is flat gray at level `10 n`, with a white top-left pixel.
fn level(n: i64) -> u8 {
    (n * 10) as u8
}

/// A lossless 24 fps clip of [`FRAMES`] flat frames.
fn write_clip(path: &Path) {
    let encoder = Encoder::start(path, VideoFormat::Ffv1, SIZE, FrameRate::whole(24)).unwrap();
    for pts in 0..FRAMES {
        let g = level(pts);
        let mut rgba = [g, g, g, 255].repeat((SIZE.0 * SIZE.1) as usize);
        rgba[..4].copy_from_slice(&[255, 255, 255, 255]);
        encoder.send(Frame { pts, rgba }).unwrap();
    }
    encoder.finish().unwrap();
}

fn load(
    path: &Path,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
) -> anyhow::Result<clip::Clip> {
    clip::load(
        path,
        pixels,
        canvas,
        budget,
        &AtomicU32::new(0),
        &AtomicBool::new(false),
    )
}

#[test]
fn clips_decode_every_frame_scaled_to_the_canvas() {
    let path = temp_file("clip-luma", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(MEMORY_LIMIT);
    let clip = load(&path, Pixels::Luma, (160, 160), &budget).unwrap();
    assert_eq!(clip.count(), FRAMES as u32);
    assert!((clip.fps - 24.0).abs() < 1e-6, "{} fps", clip.fps);
    assert_eq!(clip.format.size, (160, 90), "fitted inside the canvas");
    assert_eq!(clip.format.small, (160, 90), "never scaled up");
    for (n, frame) in clip.frames.iter().enumerate() {
        assert_eq!(frame.full.len(), 160 * 90);
        let middle = frame.full[45 * 160 + 80];
        let expected = level(n as i64);
        assert!(
            middle.abs_diff(expected) <= 2,
            "frame {n}: {middle} vs {expected}"
        );
    }
    let held = budget.clone();
    assert!(held.available() < MEMORY_LIMIT, "the clip holds its memory");
    drop(clip);
    assert_eq!(budget.available(), MEMORY_LIMIT, "and gives it back");
}

#[test]
fn background_clips_keep_their_color() {
    let path = temp_file("clip-rgba", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(MEMORY_LIMIT);
    let clip = load(&path, Pixels::Rgba, (1920, 1080), &budget).unwrap();
    assert_eq!(clip.format.size, SIZE);
    assert_eq!(clip.format.small, SIZE);
    let frame = &clip.frames[5];
    assert_eq!(frame.full.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    assert_eq!(&frame.full[..4], &[255, 255, 255, 255]);
    let px = &frame.full[(90 * SIZE.0 as usize + 160) * 4..][..4];
    for c in &px[..3] {
        assert!(c.abs_diff(50) <= 1, "{px:?}");
    }
    assert_eq!(px[3], 255);
}

#[test]
fn clips_over_the_memory_budget_are_refused() {
    let path = temp_file("clip-budget", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(1 << 20);
    let err = load(&path, Pixels::Rgba, (1920, 1080), &budget).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("clip.mkv needs 11 MB of memory; shorten it or lower the canvas size"),
        "{message}"
    );
    assert_eq!(budget.available(), 1 << 20);
}

#[test]
fn unreadable_files_name_themselves() {
    let path = temp_file("clip-bad", "broken.mp4");
    std::fs::write(&path, b"not a video").unwrap();
    let err = load(&path, Pixels::Luma, (640, 360), &Budget::new(MEMORY_LIMIT)).unwrap_err();
    assert!(
        format!("{err:#}").starts_with("could not open broken.mp4"),
        "{err:#}"
    );
}

#[test]
fn clips_load_on_a_worker_thread() {
    let path = temp_file("clip-worker", "clip.mkv");
    write_clip(&path);
    let loading = Loading::start(&path, Pixels::Luma, (320, 180), &Budget::new(MEMORY_LIMIT));
    let clip = loading.wait().unwrap();
    assert_eq!(clip.count(), FRAMES as u32);
    assert_eq!(loading.progress(), 1.0);
}
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --test video` (with the FFmpeg DLLs on `PATH`). Expected: `5 passed`. FFmpeg may print a "moov atom not found" line for the deliberately broken file; that's expected.

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `213 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `17 passed` in `tests/smoke.rs` and `5 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add src/video tests/video.rs
git commit -m "feat: decode video clips into memory on a worker thread" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 4: Cameras

Listing DirectShow cameras, and a capture thread that keeps a rolling buffer of frames with their arrival times, retrying every 3 s after the signal goes.

**Files:**
- Create: `src/video/camera.rs`
- Modify: `Cargo.toml`, `src/video/mod.rs`, `tests/video.rs`

**Interfaces:**
- Consumes: `Arrival` (Task 2); `Budget`, `Converter`, `Format`, `Frame`, `Pixels`, `Reservation` (Task 3).
- Produces (in `video::camera`):
  - `BUFFER_SECONDS: RangeInclusive<f32> = 1.0..=30.0`, `DEFAULT_BUFFER_SECONDS = 10.0`, `RETRY = 3 s`.
  - `list() -> Result<Vec<String>>` (video devices' friendly names).
  - `Status { Opening, Live, NoSignal(String) }`.
  - `Camera::open(name, pixels, canvas, buffer_seconds, &Budget) -> Camera`, with `name`, `status()`, `format() -> Option<Format>`, `note() -> Option<String>`, `arrivals() -> Vec<Arrival>`, `frame(seq) -> Option<Arc<Frame>>`; dropping it stops the thread.

- [ ] **Step 1: Make the changes.**

In `Cargo.toml`, replace:

```toml
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }
```

with:

```toml
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "device", "format", "software-scaling"] }
```

Create `src/video/camera.rs`:

```rust
//! Cameras: a DirectShow device read on a worker thread into a rolling buffer of its
//! most recent frames. The render loop only ever looks at the buffer, so it never waits
//! for the camera.

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;

use super::playhead::Arrival;
use super::{Budget, Converter, Format, Frame, Pixels, Reservation};

/// How many seconds of frames a camera keeps, for delay and slit-scan.
pub const BUFFER_SECONDS: RangeInclusive<f32> = 1.0..=30.0;
pub const DEFAULT_BUFFER_SECONDS: f32 = 10.0;

/// How long to wait before trying a camera that stopped again.
pub const RETRY: Duration = Duration::from_secs(3);

/// A camera's frame rate when the device doesn't report one.
const FALLBACK_FPS: f64 = 30.0;

/// DirectShow, FFmpeg's Windows capture input.
fn dshow() -> Result<ff::Format> {
    ff::init().context("could not initialise FFmpeg")?;
    ff::device::register_all();
    ff::device::input::video()
        .find(|f| f.name() == "dshow")
        .ok_or_else(|| anyhow!("this FFmpeg build has no DirectShow input"))
}

/// The names of the video capture devices DirectShow lists.
pub fn list() -> Result<Vec<String>> {
    let ff::Format::Input(format) = dshow()? else {
        bail!("DirectShow is not an input format");
    };
    let mut names = Vec::new();
    // SAFETY: the list is filled in and freed by FFmpeg; every pointer read here lives in
    // it until `avdevice_free_list_devices`.
    unsafe {
        use ff::ffi::*;
        let mut list: *mut AVDeviceInfoList = std::ptr::null_mut();
        let found = avdevice_list_input_sources(
            format.as_ptr() as *const _,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut list,
        );
        if found < 0 {
            avdevice_free_list_devices(&mut list);
            return Err(ff::Error::from(found)).context("could not list the cameras");
        }
        for i in 0..(*list).nb_devices.max(0) as usize {
            let device = *(*list).devices.add(i);
            let types = std::slice::from_raw_parts(
                (*device).media_types,
                (*device).nb_media_types.max(0) as usize,
            );
            if types.contains(&AVMediaType::AVMEDIA_TYPE_VIDEO) {
                let name = std::ffi::CStr::from_ptr((*device).device_description);
                names.push(name.to_string_lossy().into_owned());
            }
        }
        avdevice_free_list_devices(&mut list);
    }
    Ok(names)
}

/// How the camera is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    Live,
    /// It stopped or couldn't be opened; it's tried again every [`RETRY`]. The text says
    /// why.
    NoSignal(String),
}

/// A buffered frame and when it arrived.
pub struct Buffered {
    pub arrival: Arrival,
    pub frame: Arc<Frame>,
}

/// What the capture thread shares with the app.
struct Shared {
    status: Status,
    format: Option<Format>,
    frames: VecDeque<Buffered>,
    /// Said when the buffer had to be shorter than asked.
    note: Option<String>,
    /// The buffer's memory; kept while there's no signal, so the frames stay.
    memory: Option<Reservation>,
}

/// An open camera. Dropping it stops the capture thread.
pub struct Camera {
    pub name: String,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Camera {
    /// Starts capturing from the device called `name`, keeping `buffer_seconds` of
    /// frames for a `canvas`-sized canvas.
    pub fn open(
        name: &str,
        pixels: Pixels,
        canvas: (u32, u32),
        buffer_seconds: f32,
        budget: &Budget,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::Opening,
            format: None,
            frames: VecDeque::new(),
            note: None,
            memory: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let capture = Capture {
            name: name.to_string(),
            pixels,
            canvas,
            buffer_seconds: buffer_seconds.clamp(*BUFFER_SECONDS.start(), *BUFFER_SECONDS.end()),
            budget: budget.clone(),
            shared: shared.clone(),
            stop: stop.clone(),
            epoch: Instant::now(),
            next_seq: 0,
        };
        let spawned = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || capture.run());
        if let Err(err) = spawned {
            lock(&shared).status = Status::NoSignal(format!("could not start the camera: {err}"));
        }
        Self {
            name: name.to_string(),
            shared,
            stop,
        }
    }

    pub fn status(&self) -> Status {
        lock(&self.shared).status.clone()
    }

    /// The buffered frames' format, once the camera has opened.
    pub fn format(&self) -> Option<Format> {
        lock(&self.shared).format
    }

    /// A note about the buffer, if it had to be shorter than asked.
    pub fn note(&self) -> Option<String> {
        lock(&self.shared).note.clone()
    }

    /// When each buffered frame arrived, oldest first.
    pub fn arrivals(&self) -> Vec<Arrival> {
        lock(&self.shared)
            .frames
            .iter()
            .map(|b| b.arrival)
            .collect()
    }

    /// Frame number `seq`, if it's still buffered.
    pub fn frame(&self, seq: i64) -> Option<Arc<Frame>> {
        let shared = lock(&self.shared);
        let first = shared.frames.front()?.arrival.seq;
        let i = usize::try_from(seq - first).ok()?;
        shared
            .frames
            .get(i)
            .filter(|b| b.arrival.seq == seq)
            .map(|b| b.frame.clone())
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        // The thread notices at its next frame or retry and exits on its own.
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The capture thread's state.
struct Capture {
    name: String,
    pixels: Pixels,
    canvas: (u32, u32),
    buffer_seconds: f32,
    budget: Budget,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    epoch: Instant,
    /// Frame numbers keep counting across reconnections.
    next_seq: i64,
}

impl Capture {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Captures until stopped, retrying after each failure.
    fn run(mut self) {
        while !self.stopped() {
            let err = match self.capture() {
                Ok(()) => return,
                Err(err) => err,
            };
            log::warn!("camera {}: {err:#}", self.name);
            lock(&self.shared).status = Status::NoSignal(format!("{err:#}"));
            let retry = Instant::now() + RETRY;
            while Instant::now() < retry {
                if self.stopped() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            lock(&self.shared).status = Status::Opening;
        }
    }

    /// Opens the camera and buffers its frames. Ok when stopped; an error when it fails
    /// to open or stops sending pictures.
    fn capture(&mut self) -> Result<()> {
        let device = format!("video={}", self.name);
        let mut input = ff::format::open_with(&device, &dshow()?, ff::Dictionary::new())
            .with_context(|| format!("could not open {}", self.name))?
            .input();
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or_else(|| anyhow!("{} sends no video", self.name))?;
        let index = stream.index();
        let rate = stream.avg_frame_rate();
        let fps = if rate.numerator() > 0 && rate.denominator() > 0 {
            f64::from(rate)
        } else {
            FALLBACK_FPS
        };
        let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .video()?;
        let format = Format::new(
            self.pixels,
            (decoder.width(), decoder.height()),
            self.canvas,
        );
        let capacity = self.reserve(format, fps)?;
        let mut converter = Converter::new(format);
        let mut decoded = ff::frame::Video::empty();
        for (stream, packet) in input.packets() {
            if self.stopped() {
                return Ok(());
            }
            if stream.index() != index {
                continue;
            }
            decoder.send_packet(&packet)?;
            while decoder.receive_frame(&mut decoded).is_ok() {
                let frame = Arc::new(converter.convert(&decoded)?);
                let arrival = Arrival {
                    seq: self.next_seq,
                    time: self.epoch.elapsed().as_secs_f64(),
                };
                self.next_seq += 1;
                let mut shared = lock(&self.shared);
                shared.status = Status::Live;
                shared.frames.push_back(Buffered { arrival, frame });
                while shared.frames.len() > capacity {
                    shared.frames.pop_front();
                }
            }
        }
        if self.stopped() {
            return Ok(());
        }
        Err(anyhow!("{} stopped sending pictures", self.name))
    }

    /// Makes room for the buffer in the budget: the asked-for length, or what fits.
    /// Returns how many frames it holds. A reopened camera starts a new buffer.
    fn reserve(&self, format: Format, fps: f64) -> Result<usize> {
        let mut shared = lock(&self.shared);
        shared.frames.clear();
        shared.memory = None;
        let wanted = (f64::from(self.buffer_seconds) * fps).ceil().max(2.0) as u64;
        let fits = self.budget.available() / format.frame_bytes().max(1);
        let frames = wanted.min(fits);
        if frames < 2 {
            bail!("not enough memory left for the {} buffer", self.name);
        }
        shared.memory = self.budget.take(frames * format.frame_bytes());
        if shared.memory.is_none() {
            bail!("not enough memory left for the {} buffer", self.name);
        }
        shared.note = (frames < wanted).then(|| {
            format!(
                "Buffer shortened to {:.1} s to fit in memory",
                frames as f64 / fps
            )
        });
        shared.format = Some(format);
        Ok(frames as usize)
    }
}
```

In `src/video/mod.rs`, replace:

```rust

pub mod clip;
```

with:

```rust

pub mod camera;
pub mod clip;
```

In `tests/video.rs`, replace:

```rust
use rasterwarp::rate::FrameRate;
use rasterwarp::video::clip::{self, Loading};
```

with:

```rust
use rasterwarp::rate::FrameRate;
use rasterwarp::video::camera::{self, Camera, Status};
use rasterwarp::video::clip::{self, Loading};
```

In `tests/video.rs`, replace:

```rust
    assert_eq!(loading.progress(), 1.0);
}
```

with:

```rust
    assert_eq!(loading.progress(), 1.0);
}

#[test]
fn cameras_deliver_frames_when_one_is_connected() {
    let names = camera::list().unwrap();
    let Some(name) = names.first() else {
        eprintln!("skipping camera test: no camera connected");
        return;
    };
    let budget = Budget::new(MEMORY_LIMIT);
    let cam = Camera::open(name, Pixels::Luma, (320, 180), 2.0, &budget);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while cam.arrivals().len() < 3 {
        assert!(
            std::time::Instant::now() < deadline,
            "no frames: {:?}",
            cam.status()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(cam.status(), Status::Live);
    let format = cam.format().unwrap();
    assert!(format.size.0 <= 320 && format.size.1 <= 180, "{format:?}");
    let arrivals = cam.arrivals();
    let newest = arrivals.last().unwrap();
    assert!(
        arrivals
            .windows(2)
            .all(|w| w[1].seq == w[0].seq + 1 && w[1].time >= w[0].time)
    );
    let frame = cam.frame(newest.seq).unwrap();
    assert_eq!(frame.full.len() as u64, format.bytes_at(format.size));
    assert!(cam.frame(newest.seq + 1).is_none());
    assert!(
        budget.available() < MEMORY_LIMIT,
        "the buffer holds its memory"
    );
}
```

- [ ] **Step 2: Run the camera test.** Run: `cargo test --test video cameras -- --nocapture`. Expected: `1 passed`. With a camera connected it takes a second or two and prints nothing; without one it prints "skipping camera test: no camera connected".

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `213 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `17 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/video tests/video.rs
git commit -m "feat: cameras with a rolling frame buffer" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 5: The frames pass

A full-screen pass that draws an input's picture from a texture-array ring of its frames: the nearest frame or a blend of two, and for slit-scan a moment per pixel from rows, columns or a map image.

**Files:**
- Create: `shaders/frames.wgsl`, `src/passes/frames.rs`
- Modify: `src/gpu.rs`, `src/passes/mod.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `Sample`, `layer` (Task 2); `Pixels` (Task 3); `GrayImage`, `source::upload`.
- Produces:
  - `gpu::FullscreenPass::with_dimensions(device, &PassDesc, &[wgpu::TextureViewDimension]) -> FullscreenPass`.
  - `passes::frames::{FramesUniforms, texture_format(Pixels) -> wgpu::TextureFormat, uniforms(&Sample, layers) -> FramesUniforms, FramesPass}`.
  - `FramesPass::new(device, queue, pixels, size, max_layers)`, `pixels()`, `size()`, `layers()`, `set_map(device, queue, Option<&GrayImage>)`, `missing(device, &Sample) -> Vec<i64>` (grows the ring as needed), `upload(queue, k, &[u8])`, `draw(device, queue, encoder, &Sample)`, `pub target: RenderTarget`.

- [ ] **Step 1: Make the changes.**

Create `shaders/frames.wgsl`:

```wgsl
// Frames: draws an input's picture from its ring of buffered frames. Each pixel finds
// the moment it shows (the playhead, or further back for slit-scan) as a fractional
// frame index, then shows the nearest frame or blends the two either side.

struct Frames {
    // x: the playhead, as frames after the window's first frame
    // y: frames behind the playhead at the far end of the slit-scan map (signed)
    // z: the window's last frame, after its first
    // w: unused
    playhead: vec4<f32>,
    // x: the ring layer holding the window's first frame
    // y: layers in the ring
    // z: slit-scan: 0 off, 1 rows, 2 columns, 3 map
    // w: bit 0 flips the map, bit 1 blends between frames
    ring: vec4<u32>,
};

@group(0) @binding(0) var<uniform> u: Frames;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var frames: texture_2d_array<f32>;
@group(0) @binding(3) var slit_map: texture_2d<f32>;

// How far back this pixel looks, 0 (now) to 1 (the full depth).
fn slit_amount(uv: vec2<f32>) -> f32 {
    var m = 0.0;
    switch u.ring.z {
        case 1u: { m = uv.y; }
        case 2u: { m = uv.x; }
        case 3u: { m = textureSampleLevel(slit_map, samp, uv, 0.0).r; }
        default: { return 0.0; }
    }
    if (u.ring.w & 1u) != 0u {
        m = 1.0 - m;
    }
    return m;
}

// Frame `i` of the window (0 = its first frame).
fn frame(uv: vec2<f32>, i: f32) -> vec4<f32> {
    let layer = (u.ring.x + u32(i)) % u.ring.y;
    return textureSampleLevel(frames, samp, uv, i32(layer), 0.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let last = u.playhead.z;
    let v = clamp(u.playhead.x - u.playhead.y * slit_amount(in.uv), 0.0, last);
    if (u.ring.w & 2u) == 0u {
        return frame(in.uv, min(round(v), last));
    }
    let lo = floor(v);
    let hi = min(lo + 1.0, last);
    return mix(frame(in.uv, lo), frame(in.uv, hi), v - lo);
}
```

In `src/gpu.rs`, replace:

```rust
    pub fn new(device: &wgpu::Device, desc: &PassDesc) -> Self {
        let mut entries = vec![
```

with:

```rust
    pub fn new(device: &wgpu::Device, desc: &PassDesc) -> Self {
        let flat = vec![wgpu::TextureViewDimension::D2; desc.textures as usize];
        Self::with_dimensions(device, desc, &flat)
    }

    /// Like [`FullscreenPass::new`], with each texture's view dimension given (e.g. a
    /// 2D array); `dimensions` has one entry per texture.
    pub fn with_dimensions(
        device: &wgpu::Device,
        desc: &PassDesc,
        dimensions: &[wgpu::TextureViewDimension],
    ) -> Self {
        assert_eq!(dimensions.len(), desc.textures as usize);
        let mut entries = vec![
```

In `src/gpu.rs`, replace:

```rust
        for i in 0..desc.textures {
```

with:

```rust
        for (i, &view_dimension) in (0..).zip(dimensions) {
```

In `src/gpu.rs`, replace:

```rust
                    view_dimension: wgpu::TextureViewDimension::D2,
```

with:

```rust
                    view_dimension,
```

Create `src/passes/frames.rs`:

```rust
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
```

In `src/passes/mod.rs`, replace:

```rust
pub mod feedback;
pub mod raster;
```

with:

```rust
pub mod feedback;
pub mod frames;
pub mod raster;
```

In `tests/smoke.rs`, replace:

```rust
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::passes::composite::Area;
use rasterwarp::preview::PreviewView;
use rasterwarp::rate::FrameRate;
use rasterwarp::source::{ColorImage, GrayImage, test_card};
```

with:

```rust
use rasterwarp::params::{Between, Params, Slit};
use rasterwarp::passes::Renderer;
use rasterwarp::passes::composite::Area;
use rasterwarp::passes::frames::FramesPass;
use rasterwarp::preview::PreviewView;
use rasterwarp::rate::FrameRate;
use rasterwarp::source::{ColorImage, GrayImage, test_card};
use rasterwarp::video::Pixels;
use rasterwarp::video::playhead::Sample;
```

In `tests/smoke.rs`, replace:

```rust
/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
```

with:

```rust
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
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --test smoke -- frames slit`. Expected: `4 passed` (`the_frames_pass_shows_or_blends_neighbouring_frames`, `slit_scan_rows_show_older_frames_further_down`, `slit_scan_follows_the_map_image`, and the existing `renders_frames_offscreen`).

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `214 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `20 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add shaders/frames.wgsl src/gpu.rs src/passes tests/smoke.rs
git commit -m "feat: the frames pass: mix and slit-scan frames from a GPU ring" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 6: Renderer inputs from frames

The renderer's source and background each become an image or a frames pass; the warp, raster and composite passes sample whichever it is.

**Files:**
- Modify: `src/passes/mod.rs`, `src/preview.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `FramesPass` (Task 5), `Role`, `Pixels`, `Sample`.
- Produces:
  - `Renderer::set_frames(device, queue, role: Role, size: (u32, u32), max_layers: u32)` (a new source resets the beam motion), `Renderer::frames_mut(role) -> Option<&mut FramesPass>`, `Renderer::draw_frames(device, queue, encoder, role, &Sample)`.
  - `PreviewView::renderer_mut() -> &mut Renderer`.

- [ ] **Step 1: Make the changes.**

In `src/passes/mod.rs`, replace:

```rust
use crate::params::GlowParams;
use crate::passes::composite::Area;
use crate::source::{self, ColorImage, GrayImage};

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
```

with:

```rust
use crate::params::{GlowParams, Role};
use crate::passes::composite::Area;
use crate::passes::frames::FramesPass;
use crate::source::{self, ColorImage, GrayImage};
use crate::video::Pixels;
use crate::video::playhead::Sample;

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Where an input's picture comes from on the GPU.
enum Picture {
    Image(wgpu::TextureView),
    /// A video or camera, drawn each frame by its frames pass.
    Frames(Box<FramesPass>),
}

impl Picture {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Picture::Image(view) => view,
            Picture::Frames(pass) => &pass.target.view,
        }
    }
}
```

In `src/passes/mod.rs`, replace:

```rust
    source: wgpu::TextureView,
    source_aspect: f32,
    /// What keyed levels show.
    background: wgpu::TextureView,
```

with:

```rust
    source: Picture,
    source_aspect: f32,
    /// What keyed levels show.
    background: Picture,
```

In `src/passes/mod.rs`, replace:

```rust
            source: source::upload(device, queue, image).create_view(&Default::default()),
            source_aspect: image.aspect(),
            background: source::upload_color(device, queue, &ColorImage::black())
                .create_view(&Default::default()),
```

with:

```rust
            source: Picture::Image(
                source::upload(device, queue, image).create_view(&Default::default()),
            ),
            source_aspect: image.aspect(),
            background: Picture::Image(
                source::upload_color(device, queue, &ColorImage::black())
                    .create_view(&Default::default()),
            ),
```

In `src/passes/mod.rs`, replace:

```rust
        self.source = source::upload(device, queue, image).create_view(&Default::default());
```

with:

```rust
        self.source =
            Picture::Image(source::upload(device, queue, image).create_view(&Default::default()));
```

In `src/passes/mod.rs`, replace:

```rust
        self.background =
            source::upload_color(device, queue, image).create_view(&Default::default());
        self.background_aspect = image.aspect();
```

with:

```rust
        self.background = Picture::Image(
            source::upload_color(device, queue, image).create_view(&Default::default()),
        );
        self.background_aspect = image.aspect();
    }

    /// Makes `role`'s input a video or camera with frames of `size` pixels, drawn by a
    /// frames pass whose ring may grow to `max_layers` layers. A new source resets the
    /// beam motion, like a new image.
    pub fn set_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        role: Role,
        size: (u32, u32),
        max_layers: u32,
    ) {
        let pass = FramesPass::new(device, queue, Pixels::for_role(role), size, max_layers);
        let picture = Picture::Frames(Box::new(pass));
        let aspect = size.0 as f32 / size.1 as f32;
        match role {
            Role::Source => {
                self.source = picture;
                self.source_aspect = aspect;
                self.reset_motion();
            }
            Role::Background => {
                self.background = picture;
                self.background_aspect = aspect;
            }
        }
    }

    /// `role`'s frames pass, when its input is a video or camera.
    pub fn frames_mut(&mut self, role: Role) -> Option<&mut FramesPass> {
        let picture = match role {
            Role::Source => &mut self.source,
            Role::Background => &mut self.background,
        };
        match picture {
            Picture::Frames(pass) => Some(pass),
            Picture::Image(_) => None,
        }
    }

    /// Draws `role`'s picture for this frame from its frames pass (whose window the
    /// caller has filled; see [`FramesPass::missing`]). Nothing for an image.
    pub fn draw_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        role: Role,
        sample: &Sample,
    ) {
        if let Some(pass) = self.frames_mut(role) {
            pass.draw(device, queue, encoder, sample);
        }
```

In `src/passes/mod.rs`, replace:

```rust
                &self.source,
```

with:

```rust
                self.source.view(),
```

In `src/passes/mod.rs`, replace:

```rust
                .render(device, queue, encoder, &self.source, &deflection);
```

with:

```rust
                .render(device, queue, encoder, self.source.view(), &deflection);
```

In `src/passes/mod.rs`, replace:

```rust
            self.feedback.output(),
            self.bloom.output(),
            &self.background,
            output,
            area,
```

with:

```rust
            self.feedback.output(),
            self.bloom.output(),
            self.background.view(),
            output,
            area,
```

In `src/passes/mod.rs`, replace:

```rust
            &self.background,
```

with:

```rust
            self.background.view(),
```

In `src/preview.rs`, replace:

```rust

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

with:

```rust

    /// The preview's renderer, for its video and camera inputs (see
    /// [`Renderer::set_frames`]). Its frames are the small copies, at preview size.
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

In `tests/smoke.rs`, replace:

```rust
use rasterwarp::params::{Between, Params, Slit};
```

with:

```rust
use rasterwarp::params::{Between, Params, Role, Slit};
```

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

/// Fills `role`'s frames pass with those of `frames` in `sample`'s window and draws
/// `sample` into `encoder`.
fn feed(
    renderer: &mut Renderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    role: Role,
    frames: &[Vec<u8>],
    sample: &Sample,
) {
    let pass = renderer.frames_mut(role).expect("a frames input");
    for k in pass.missing(device, sample) {
        if let Some(frame) = frames.get(k as usize) {
            pass.upload(queue, k, frame);
        }
    }
    renderer.draw_frames(device, queue, encoder, role, sample);
}

#[test]
fn a_video_source_goes_through_the_whole_pipeline() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (128, 72);
    let mut params = flat_params();
    params.colorize.bypass = true;
    let frame = FrameParams::at_rest(&params);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "video source", size.0, size.1, format);
    let mut renderer = Renderer::new(&device, &queue, format, size, &test_card(64, 36));
    renderer.set_frames(&device, &queue, Role::Source, (64, 36), 8);
    let frames = flat_frames(&[0, 255], (64, 36));
    let mut middle = |base| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let sample = still_sample(base, Between::Nearest);
        feed(
            &mut renderer,
            &device,
            &queue,
            &mut encoder,
            Role::Source,
            &frames,
            &sample,
        );
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame,
            0.0,
            &output.view,
            size,
        );
        queue.submit([encoder.finish()]);
        read_back(&device, &queue, &output.texture)[((36 * size.0 + 64) * 4) as usize]
    };
    assert!(middle(0.0) < 10, "frame 0 is black");
    assert!(middle(1.0) > 245, "frame 1 is white");
}

#[test]
fn keyed_levels_show_a_video_background() {
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
    renderer.set_frames(&device, &queue, Role::Background, (2, 2), 8);
    let green = [0, 255, 0, 255].repeat(4);
    let frames = vec![[255, 0, 0, 255].repeat(4), green];
    let mut encoder = device.create_command_encoder(&Default::default());
    let sample = still_sample(1.0, Between::Nearest);
    feed(
        &mut renderer,
        &device,
        &queue,
        &mut encoder,
        Role::Background,
        &frames,
        &sample,
    );
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
    let i = ((size.1 / 2 * size.0 + 40) * 4) as usize;
    assert_eq!(
        &pixels[i..i + 3],
        &[0, 255, 0],
        "the keyed level shows frame 1"
    );
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --test smoke video`. Expected: `2 passed` (`a_video_source_goes_through_the_whole_pipeline`, `keyed_levels_show_a_video_background`).

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `214 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `22 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add src/passes/mod.rs src/preview.rs tests/smoke.rs
git commit -m "feat: renderer inputs drawn from frames" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 7: Projects save cameras and slit-scan maps

Projects gain each input's camera (name and buffer length) and slit-scan map, beside the existing file paths. The app keeps all of it in one `InputFiles` instead of two paths.

**Files:**
- Modify: `src/session.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `Role` (Task 1), `BUFFER_SECONDS`, `DEFAULT_BUFFER_SECONDS` (Task 4).
- Produces (in `session`):
  - `CameraChoice { name: String, buffer_seconds: f32 }` (`Default`: empty name, 10 s).
  - `InputFile { path: Option<PathBuf>, camera: Option<CameraChoice>, slit_map: Option<PathBuf> }`.
  - `InputFiles { source, background, source_camera, background_camera, source_slit_map, background_slit_map }` with `get(Role) -> InputFile` and `set(Role, InputFile)`; `Project.inputs: InputFiles` (flattened in the file).
  - `Project::capture(motion, canvas, frame_rate, inputs: &InputFiles) -> Project`.

- [ ] **Step 1: Make the changes.**

In `src/app.rs`, replace:

```rust
use crate::session::{self, Autosave, Project};
```

with:

```rust
use crate::session::{self, Autosave, InputFiles, Project};
```

In `src/app.rs`, replace:

```rust
    /// The source and background image files, as a project saves them. A project's
    /// missing image keeps its path here, so saving doesn't drop it.
    source_path: Option<PathBuf>,
    background_path: Option<PathBuf>,
```

with:

```rust
    /// The inputs' files and cameras, as a project saves them. A project's missing image
    /// keeps its path here, so saving doesn't drop it.
    inputs: InputFiles,
```

In `src/app.rs`, replace:

```rust
            source_path: None,
            background_path: None,
```

with:

```rust
            inputs: InputFiles::default(),
```

In `src/app.rs`, replace:

```rust
        match &project.source {
```

with:

```rust
        match &project.inputs.source {
```

In `src/app.rs`, replace:

```rust
        match &project.background {
```

with:

```rust
        match &project.inputs.background {
```

In `src/app.rs`, replace:

```rust
        self.source_path = project.source.clone();
        self.background_path = project.background.clone();
```

with:

```rust
        self.inputs = project.inputs.clone();
```

In `src/app.rs`, replace:

```rust
            self.source_path.as_deref(),
            self.background_path.as_deref(),
```

with:

```rust
            &self.inputs,
```

In `src/app.rs`, replace:

```rust
                self.source_path = Some(absolute(path));
```

with:

```rust
                self.inputs.source = Some(absolute(path));
```

In `src/app.rs`, replace:

```rust
                    self.background_path = Some(absolute(&path));
```

with:

```rust
                    self.inputs.background = Some(absolute(&path));
```

In `src/app.rs`, replace:

```rust
            self.background_path = None;
```

with:

```rust
            self.inputs.background = None;
```

In `src/session.rs`, replace:

```rust
//! the sequence, user curves, canvas size, frame rate and image paths, but nothing in
//! motion (ramp progress, the sequence's position, phases, trails).
```

with:

```rust
//! the sequence, user curves, canvas size, frame rate and inputs (files and cameras),
//! but nothing in motion (ramp progress, the sequence's position, phases, video clocks,
//! trails, camera buffers).
```

In `src/session.rs`, replace:

```rust
use crate::params::Params;
```

with:

```rust
use crate::params::{Params, Role};
```

In `src/session.rs`, replace:

```rust
use crate::transition::{AbState, DURATION};

```

with:

```rust
use crate::transition::{AbState, DURATION};
use crate::video::camera::{BUFFER_SECONDS, DEFAULT_BUFFER_SECONDS};

```

In `src/session.rs`, replace:

```rust
    /// The source image, or none for the built-in test card.
    pub source: Option<PathBuf>,
    /// The keying background image, if any.
    pub background: Option<PathBuf>,
```

with:

```rust
    #[serde(flatten)]
    pub inputs: InputFiles,
}

/// A camera input as a project saves it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraChoice {
    /// The device's name, as DirectShow lists it.
    pub name: String,
    /// Seconds of frames kept for delay and slit-scan.
    pub buffer_seconds: f32,
}

impl Default for CameraChoice {
    fn default() -> Self {
        Self {
            name: String::new(),
            buffer_seconds: DEFAULT_BUFFER_SECONDS,
        }
    }
}

/// One input as a project saves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputFile {
    /// An image or video file, or none (the test card, or a black background).
    pub path: Option<PathBuf>,
    /// When set, the input is this camera and `path` is ignored.
    pub camera: Option<CameraChoice>,
    /// The slit-scan map image.
    pub slit_map: Option<PathBuf>,
}

/// Both inputs' files and cameras. In the file they are top-level fields; `source` and
/// `background` are the fields older projects already have.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputFiles {
    pub source: Option<PathBuf>,
    pub background: Option<PathBuf>,
    pub source_camera: Option<CameraChoice>,
    pub background_camera: Option<CameraChoice>,
    pub source_slit_map: Option<PathBuf>,
    pub background_slit_map: Option<PathBuf>,
}

impl InputFiles {
    pub fn get(&self, role: Role) -> InputFile {
        let (path, camera, slit_map) = match role {
            Role::Source => (&self.source, &self.source_camera, &self.source_slit_map),
            Role::Background => (
                &self.background,
                &self.background_camera,
                &self.background_slit_map,
            ),
        };
        InputFile {
            path: path.clone(),
            camera: camera.clone(),
            slit_map: slit_map.clone(),
        }
    }

    pub fn set(&mut self, role: Role, input: InputFile) {
        let (path, camera, slit_map) = match role {
            Role::Source => (
                &mut self.source,
                &mut self.source_camera,
                &mut self.source_slit_map,
            ),
            Role::Background => (
                &mut self.background,
                &mut self.background_camera,
                &mut self.background_slit_map,
            ),
        };
        *path = input.path;
        *camera = input.camera;
        *slit_map = input.slit_map;
    }

    /// A copy with each camera's buffer length inside its range.
    fn checked(&self) -> Self {
        let mut inputs = self.clone();
        for camera in [&mut inputs.source_camera, &mut inputs.background_camera]
            .into_iter()
            .flatten()
        {
            camera.buffer_seconds = camera
                .buffer_seconds
                .clamp(*BUFFER_SECONDS.start(), *BUFFER_SECONDS.end());
        }
        inputs
    }
```

In `src/session.rs`, replace:

```rust
            source: None,
            background: None,
```

with:

```rust
            inputs: InputFiles::default(),
```

In `src/session.rs`, replace:

```rust
        source: Option<&Path>,
        background: Option<&Path>,
```

with:

```rust
        inputs: &InputFiles,
```

In `src/session.rs`, replace:

```rust
            source: source.map(Path::to_path_buf),
            background: background.map(Path::to_path_buf),
        }
    }

    /// The motion this project describes, at rest. The canvas, frame rate and images are
    /// the app's to apply.
```

with:

```rust
            inputs: inputs.clone(),
        }
    }

    /// The motion this project describes, at rest. The canvas, frame rate and inputs
    /// are the app's to apply.
```

In `src/session.rs`, replace:

```rust
            self.source.as_deref(),
            self.background.as_deref(),
```

with:

```rust
            &self.inputs.checked(),
```

In `src/session.rs`, replace:

```rust
        Project::capture(
            &session(),
            (1280, 720),
            FrameRate::ntsc(24),
            Some(Path::new(r"C:\images\card.png")),
            None,
        )
```

with:

```rust
        let inputs = InputFiles {
            source: Some(PathBuf::from(r"C:\images\card.png")),
            ..InputFiles::default()
        };
        Project::capture(&session(), (1280, 720), FrameRate::ntsc(24), &inputs)
```

In `src/session.rs`, replace:

```rust
        let p = Project {
            background: Some(PathBuf::from(r"C:\images\key.png")),
            ..project()
        };
```

with:

```rust
        let mut p = project();
        p.inputs.background = Some(PathBuf::from(r"C:\images\key.png"));
```

In `src/session.rs`, replace:

```rust
        let p = Project::capture(&m, (1280, 720), FrameRate::ntsc(24), None, None);
        let mut restored = p.motion();
        assert_eq!(
            Project::capture(&restored, (1280, 720), FrameRate::ntsc(24), None, None),
```

with:

```rust
        let p = Project::capture(&m, (1280, 720), FrameRate::ntsc(24), &InputFiles::default());
        let mut restored = p.motion();
        assert_eq!(
            Project::capture(
                &restored,
                (1280, 720),
                FrameRate::ntsc(24),
                &InputFiles::default()
            ),
```

In `src/session.rs`, replace:

```rust
        let before = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
        assert_eq!(
            Project::capture(&m, (1280, 720), FrameRate::default(), None, None),
```

with:

```rust
        let before = Project::capture(
            &m,
            (1280, 720),
            FrameRate::default(),
            &InputFiles::default(),
        );
        assert_eq!(
            Project::capture(
                &m,
                (1280, 720),
                FrameRate::default(),
                &InputFiles::default()
            ),
```

In `src/session.rs`, replace:

```rust
        let after = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
        assert_ne!(after, before);
        m.trigger();
        m.advance(1000);
        let running = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
```

with:

```rust
        let after = Project::capture(
            &m,
            (1280, 720),
            FrameRate::default(),
            &InputFiles::default(),
        );
        assert_ne!(after, before);
        m.trigger();
        m.advance(1000);
        let running = Project::capture(
            &m,
            (1280, 720),
            FrameRate::default(),
            &InputFiles::default(),
        );
```

In `src/session.rs`, replace:

```rust
        assert_eq!(p.source.as_deref(), Some(Path::new(r"C:\images\card.png")));
        assert_eq!(p.background, None);
```

with:

```rust
        let card = Some(PathBuf::from(r"C:\images\card.png"));
        assert_eq!(
            p.inputs,
            InputFiles {
                source: card,
                ..InputFiles::default()
            },
            "no cameras or slit-scan maps"
        );
    }

    #[test]
    fn cameras_and_slit_scan_maps_round_trip() {
        let mut p = project();
        p.inputs.set(
            Role::Background,
            InputFile {
                path: Some(PathBuf::from(r"C:\clips\loop.mp4")),
                camera: Some(CameraChoice {
                    name: "Logi C270 HD WebCam".into(),
                    buffer_seconds: 4.0,
                }),
                slit_map: Some(PathBuf::from(r"C:\images\map.png")),
            },
        );
        let json: Value = serde_json::from_str(&project_json(&p).unwrap()).unwrap();
        assert_eq!(json["background"], json!(r"C:\clips\loop.mp4"));
        assert_eq!(
            json["background_camera"]["name"],
            json!("Logi C270 HD WebCam")
        );
        assert_eq!(json["background_slit_map"], json!(r"C:\images\map.png"));
        let loaded = read_project(&project_json(&p).unwrap()).unwrap().value;
        assert_eq!(loaded, p);
        let background = loaded.inputs.get(Role::Background);
        assert_eq!(background.camera.unwrap().buffer_seconds, 4.0);
        assert_eq!(loaded.inputs.get(Role::Source).camera, None);
    }

    #[test]
    fn camera_buffers_are_kept_in_range() {
        let p =
            project_with(|j| j["source_camera"] = json!({"name": "Cam", "buffer_seconds": 99.0}));
        let camera = p.inputs.source_camera.unwrap();
        assert_eq!((camera.name.as_str(), camera.buffer_seconds), ("Cam", 30.0));
        let p = project_with(|j| j["source_camera"] = json!({"name": "Cam"}));
        assert_eq!(p.inputs.source_camera.unwrap().buffer_seconds, 10.0);
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --lib session`. Expected: all pass, among them `cameras_and_slit_scan_maps_round_trip`, `camera_buffers_are_kept_in_range` and `the_version_1_project_still_loads`.

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `216 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `22 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/app.rs
git commit -m "feat: projects save input cameras and slit-scan maps" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 8: Play clips and cameras in the app

The app's inputs as they run: a dropped (or command-line) video, or one in a project, loads in the background and then plays as the source or background; a project's camera opens; each canvas frame feeds both renderers from the playhead.

**Files:**
- Create: `src/inputs.rs`
- Modify: `src/lib.rs`, `src/app.rs`, `src/passes/frames.rs`, `src/preview.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `inputs::Kind { Image, Video, Camera }` with `ALL` and `label()`; `inputs::is_image(&Path) -> bool`; `inputs::View { Main, Preview }`; `inputs::describe_clip(&Path, &Clip) -> String`.
  - `inputs::Input::new(Role)` with `role`, `pub info: String`, `kind()`, `loading() -> Option<(&Path, f32)>`, `camera_status() -> Option<Status>`, `camera_note()`, `max_depth(&mut Renderer) -> Option<f32>`, `load(path, canvas, &Budget)`, `show_still(info: String)`, `open_camera(&CameraChoice, canvas, &Budget)`, `set_map(device, queue, [&mut Renderer; 2], Option<GrayImage>)`, `poll(device, queue, [&mut Renderer; 2]) -> Option<Result<PathBuf>>`, `sync_preview()`, `feed(device, queue, encoder, &mut Renderer, View, &VideoFrame, paused: bool)`.
  - `FramesPass::max_layers()`, `FramesPass::forget()`; `PreviewView::shows() -> Option<PreviewSource>`.

- [ ] **Step 1: Make the changes.**

In `src/app.rs`, replace:

```rust
use crate::motion::{Mode, Motion};
use crate::params::Params;
```

with:

```rust
use crate::inputs::{Input, View, is_image};
use crate::motion::{Mode, Motion};
use crate::params::{Params, Role};
```

In `src/app.rs`, replace:

```rust
use crate::session::{self, Autosave, InputFiles, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};
```

with:

```rust
use crate::session::{self, Autosave, InputFile, InputFiles, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};
use crate::video::{Budget, MEMORY_LIMIT};
```

In `src/app.rs`, replace:

```rust
    inputs: InputFiles,
    /// The named project file the session belongs to; none while Untitled.
```

with:

```rust
    inputs: InputFiles,
    /// The inputs as they run (clips, cameras), indexed by [`Role::index`].
    feeds: [Input; 2],
    /// The memory clips and camera buffers share.
    budget: Budget,
    /// The named project file the session belongs to; none while Untitled.
```

In `src/app.rs`, replace:

```rust
            inputs: InputFiles::default(),
            project_file: None,
```

with:

```rust
            inputs: InputFiles::default(),
            feeds: Role::ALL.map(Input::new),
            budget: Budget::new(MEMORY_LIMIT),
            project_file: None,
```

In `src/app.rs`, replace:

```rust
    /// and images. A missing image is reported and the test card (or no background) is
    /// shown instead, but its path stays in the session.
```

with:

```rust
    /// and inputs. A missing file is reported and the test card (or no background) is
    /// shown instead, but its path stays in the session. Clips show once loaded, and
    /// cameras once they send a picture.
```

In `src/app.rs`, replace:

```rust
        match &project.inputs.source {
            Some(path) => {
                if let Err(err) = self.open_source(path) {
                    problems.push(image_problem("Source", path, &err));
                    self.show_test_card();
                }
            }
            None => self.show_test_card(),
        }
        match &project.inputs.background {
            Some(path) => {
                if let Err(err) = self.open_background(path) {
                    problems.push(image_problem("Background", path, &err));
                    self.set_background(None);
                }
            }
            None => self.set_background(None),
```

with:

```rust
        for role in Role::ALL {
            self.apply_input(role, &project.inputs.get(role), &mut problems);
```

In `src/app.rs`, replace:

```rust
    /// Loads a dropped (or command-line) image as the source.
    fn load_source(&mut self, path: &Path) {
        match self.open_source(path) {
            Ok(()) => {
                self.inputs.source = Some(absolute(path));
```

with:

```rust
    /// Loads a dropped (or command-line) image or video as the source.
    fn load_source(&mut self, path: &Path) {
        self.open_file(Role::Source, path);
    }

    /// Opens `path` as `role`'s input: an image at once, or a clip once it has loaded
    /// (see [`State::poll_inputs`]); until then the input shows what it did. The project
    /// remembers the file once it shows.
    fn open_file(&mut self, role: Role, path: &Path) {
        if !is_image(path) {
            self.feeds[role.index()].load(path, self.renderer.size(), &self.budget);
            return;
        }
        let opened = match role {
            Role::Source => self.open_source(path),
            Role::Background => self.open_background(path),
        };
        match opened {
            Ok(()) => {
                self.remember_file(role, Some(absolute(path)));
```

In `src/app.rs`, replace:

```rust

    /// Shows the image at `path` as the source.
```

with:

```rust

    /// Records that `role`'s input is the file at `path` (or nothing), not a camera.
    fn remember_file(&mut self, role: Role, path: Option<PathBuf>) {
        let mut file = self.inputs.get(role);
        file.path = path;
        file.camera = None;
        self.inputs.set(role, file);
    }

    /// Takes over clips that finished loading, and reports those that failed.
    fn poll_inputs(&mut self) {
        for role in Role::ALL {
            let renderers = [&mut self.renderer, self.preview.renderer_mut()];
            let feed = &mut self.feeds[role.index()];
            match feed.poll(&self.device, &self.queue, renderers) {
                Some(Ok(path)) => {
                    let info = format!("{}: {}", role.label(), feed.info);
                    match role {
                        Role::Source => self.ui.source_info = info,
                        Role::Background => self.ui.background_info = Some(info),
                    }
                    self.remember_file(role, Some(absolute(&path)));
                    self.ui.load_error = None;
                }
                Some(Err(err)) => {
                    log::warn!("{err:#}");
                    self.ui.load_error = Some(format!("{err:#}"));
                }
                None => {}
            }
        }
    }

    /// Shows the image at `path` as the source.
```

In `src/app.rs`, replace:

```rust
        self.ui.source_info = describe(&path.display().to_string(), &image);
        Ok(())
```

with:

```rust
        self.ui.source_info = describe(&path.display().to_string(), &image);
        self.feeds[Role::Source.index()].show_still(self.ui.source_info.clone());
        Ok(())
```

In `src/app.rs`, replace:

```rust
    }

    /// Asks for a background image for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            match self.open_background(&path) {
                Ok(()) => {
                    self.inputs.background = Some(absolute(&path));
                    self.ui.load_error = None;
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    self.ui.load_error = Some(format!("{err:#}"));
                }
            }
```

with:

```rust
        self.feeds[Role::Source.index()].show_still(self.ui.source_info.clone());
    }

    /// Asks for a background image or video for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image or video")
            .add_filter("Images and videos", MEDIA_EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.open_file(Role::Background, &path);
```

In `src/app.rs`, replace:

```rust
            self.ui.background_info = None;
        }
```

with:

```rust
            self.ui.background_info = None;
        }
        self.feeds[Role::Background.index()].show_still(String::new());
    }

    /// Loads `role`'s slit-scan map image, or clears it.
    fn open_map(&mut self, role: Role, path: Option<&Path>) -> Result<()> {
        let map = path
            .map(|path| load_fitting(path, self.max_texture_side))
            .transpose();
        let renderers = [&mut self.renderer, self.preview.renderer_mut()];
        let feed = &mut self.feeds[role.index()];
        match map {
            Ok(map) => {
                feed.set_map(&self.device, &self.queue, renderers, map);
                Ok(())
            }
            Err(err) => {
                feed.set_map(&self.device, &self.queue, renderers, None);
                Err(err)
            }
        }
    }

    /// Makes `role` show what `file` names, as opening a project does. Problems are
    /// added to `problems`, and the input shows nothing (the test card, or black).
    fn apply_input(&mut self, role: Role, file: &InputFile, problems: &mut Vec<String>) {
        let label = role.label();
        // Until a clip or camera shows, the input shows nothing.
        match role {
            Role::Source => self.show_test_card(),
            Role::Background => self.set_background(None),
        }
        if let Err(err) = self.open_map(role, file.slit_map.as_deref()) {
            let path = file.slit_map.as_deref().unwrap_or(Path::new(""));
            problems.push(file_problem(&format!("{label} slit-scan map"), path, &err));
        }
        if let Some(camera) = &file.camera {
            let canvas = self.renderer.size();
            let feed = &mut self.feeds[role.index()];
            feed.open_camera(camera, canvas, &self.budget);
            return;
        }
        let Some(path) = &file.path else {
            return;
        };
        if !is_image(path) {
            if path.exists() {
                let canvas = self.renderer.size();
                self.feeds[role.index()].load(path, canvas, &self.budget);
            } else {
                problems.push(format!("{label} video not found: {}", path.display()));
            }
            return;
        }
        let opened = match role {
            Role::Source => self.open_source(path),
            Role::Background => self.open_background(path),
        };
        if let Err(err) = opened {
            problems.push(file_problem(&format!("{label} image"), path, &err));
            match role {
                Role::Source => self.show_test_card(),
                Role::Background => self.set_background(None),
            }
        }
```

In `src/app.rs`, replace:

```rust
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.render_canvas(
```

with:

```rust
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for (feed, video) in self.feeds.iter_mut().zip(&self.frame_params.video) {
            let renderer = &mut self.renderer;
            feed.feed(
                &self.device,
                &self.queue,
                &mut encoder,
                renderer,
                View::Main,
                video,
                paused,
            );
        }
        self.renderer.render_canvas(
```

In `src/app.rs`, replace:

```rust
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
```

with:

```rust
            Some(preview) => {
                if self.preview.shows() != Some(preview.source) {
                    for feed in &mut self.feeds {
                        feed.sync_preview();
                    }
                }
                for (feed, video) in self.feeds.iter_mut().zip(&preview.frame.video) {
                    feed.feed(
                        &self.device,
                        &self.queue,
                        &mut encoder,
                        self.preview.renderer_mut(),
                        View::Preview,
                        video,
                        paused,
                    );
                }
                self.preview.render(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &preview,
                    self.time as f32,
                );
            }
```

In `src/app.rs`, replace:

```rust
            });
        self.ui.late = self.clock.late();
```

with:

```rust
            });
        self.poll_inputs();
        self.ui.late = self.clock.late();
```

In `src/app.rs`, replace:

```rust
            self.inputs.background = None;
```

with:

```rust
            self.remember_file(Role::Background, None);
```

In `src/app.rs`, replace:

```rust
/// What to say about a project's image that didn't load.
fn image_problem(kind: &str, path: &Path, err: &anyhow::Error) -> String {
    if path.exists() {
        format!("{err:#}")
    } else {
        format!("{kind} image not found: {}", path.display())
    }
}
```

with:

```rust
/// What to say about a project's file (`what`, e.g. "Source image") that didn't load.
fn file_problem(what: &str, path: &Path, err: &anyhow::Error) -> String {
    if path.exists() {
        format!("{err:#}")
    } else {
        format!("{what} not found: {}", path.display())
    }
}

/// What the open dialogs offer: the images the app reads and common video files.
const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "mp4", "mov", "mkv", "avi", "webm", "m4v",
];
```

Create `src/inputs.rs`:

```rust
//! The app's two inputs (the source and the background) while it runs: whether each
//! shows an image, a clip or a camera; loading clips and opening cameras; and feeding
//! their frames to the renderers every canvas frame.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::blend::VideoFrame;
use crate::params::{PlayMode, Role, Slit};
use crate::passes::Renderer;
use crate::session::CameraChoice;
use crate::source::GrayImage;
use crate::video::camera::{Camera, Status};
use crate::video::clip::{Clip, Loading};
use crate::video::playhead::{self, Playback, camera_sample, clip_frame, clip_sample, max_layers};
use crate::video::{Budget, Format, Pixels};

/// What an input can be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// An image, the built-in test card, or (for the background) black.
    #[default]
    Image,
    Video,
    Camera,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Image, Kind::Video, Kind::Camera];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "Image",
            Kind::Video => "Video",
            Kind::Camera => "Camera",
        }
    }
}

/// Whether a file opens as an image (by its extension) rather than as a video.
pub fn is_image(path: &Path) -> bool {
    image::ImageFormat::from_path(path).is_ok()
}

/// Which renderer a picture is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The canvas, from the frames' full-size copies.
    Main,
    /// The off-air preview, from their small copies.
    Preview,
}

impl View {
    fn index(self) -> usize {
        match self {
            View::Main => 0,
            View::Preview => 1,
        }
    }
}

enum Content {
    /// An image, already on the renderers.
    Still,
    Clip(Clip),
    Camera(Camera),
}

/// One input as it runs.
pub struct Input {
    pub role: Role,
    content: Content,
    /// A clip being decoded; what's showing stays until it's ready.
    loading: Option<Loading>,
    /// Each view's clip playback.
    playback: [Playback; 2],
    /// The play mode each view's frame ring was filled for.
    modes: [Option<PlayMode>; 2],
    /// The format the renderers' frames passes were made for.
    format: Option<Format>,
    map: Option<GrayImage>,
    /// Where a paused camera stopped (an arrival time).
    held: Option<f64>,
    /// What's showing, for the panel.
    pub info: String,
}

/// `fps` as people write it: "24" or "29.97".
fn describe_fps(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.005 {
        format!("{}", fps.round())
    } else {
        format!("{fps:.2}")
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The panel's description of a clip.
pub fn describe_clip(path: &Path, clip: &Clip) -> String {
    let (w, h) = clip.format.size;
    format!(
        "{}: {w}×{h}, {} frames at {} fps",
        file_name(path),
        clip.count(),
        describe_fps(clip.fps)
    )
}

impl Input {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            content: Content::Still,
            loading: None,
            playback: [Playback::default(); 2],
            modes: [None; 2],
            format: None,
            map: None,
            held: None,
            info: String::new(),
        }
    }

    pub fn kind(&self) -> Kind {
        match self.content {
            Content::Still => Kind::Image,
            Content::Clip(_) => Kind::Video,
            Content::Camera(_) => Kind::Camera,
        }
    }

    /// The clip being loaded and how far along it is (0–1).
    pub fn loading(&self) -> Option<(&Path, f32)> {
        self.loading
            .as_ref()
            .map(|l| (l.path.as_path(), l.progress()))
    }

    /// The camera's state, when the input is a camera.
    pub fn camera_status(&self) -> Option<Status> {
        match &self.content {
            Content::Camera(camera) => Some(camera.status()),
            _ => None,
        }
    }

    /// A note about the camera's buffer, when it had to be shorter than asked.
    pub fn camera_note(&self) -> Option<String> {
        match &self.content {
            Content::Camera(camera) => camera.note(),
            _ => None,
        }
    }

    /// The deepest slit-scan the canvas's frame ring allows, in seconds.
    pub fn max_depth(&self, renderer: &mut Renderer) -> Option<f32> {
        let fps = match &self.content {
            Content::Still => return None,
            Content::Clip(clip) => clip.fps,
            Content::Camera(camera) => playhead::camera_fps(&camera.arrivals()),
        };
        let layers = renderer.frames_mut(self.role)?.max_layers();
        Some(playhead::max_depth(layers, fps))
    }

    /// Starts decoding the clip at `path` for a `canvas`-sized canvas, replacing any
    /// load in progress.
    pub fn load(&mut self, path: &Path, canvas: (u32, u32), budget: &Budget) {
        self.loading = Some(Loading::start(
            path,
            Pixels::for_role(self.role),
            canvas,
            budget,
        ));
    }

    /// Shows an image (which the caller has put on the renderers), stopping any clip,
    /// camera or load.
    pub fn show_still(&mut self, info: String) {
        self.content = Content::Still;
        self.loading = None;
        self.format = None;
        self.info = info;
    }

    /// Switches to the camera `choice`. Its picture appears with its first frame.
    pub fn open_camera(&mut self, choice: &CameraChoice, canvas: (u32, u32), budget: &Budget) {
        self.loading = None;
        // The old camera's memory goes back first, so the new buffer can use it.
        self.content = Content::Still;
        self.format = None;
        let camera = Camera::open(
            &choice.name,
            Pixels::for_role(self.role),
            canvas,
            choice.buffer_seconds,
            budget,
        );
        self.content = Content::Camera(camera);
        self.info = format!("{} (camera)", choice.name);
    }

    /// Uses `map` for slit-scan's Map, or Rows without one.
    pub fn set_map(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderers: [&mut Renderer; 2],
        map: Option<GrayImage>,
    ) {
        self.map = map;
        for renderer in renderers {
            if let Some(pass) = renderer.frames_mut(self.role) {
                pass.set_map(device, queue, self.map.as_ref());
            }
        }
    }

    /// Makes the renderers' frames passes for frames in `format`.
    fn make_passes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        [main, preview]: [&mut Renderer; 2],
        format: Format,
    ) {
        let device_layers = device.limits().max_texture_array_layers;
        for (renderer, size) in [(main, format.size), (preview, format.small)] {
            let layers = max_layers(device_layers, format.bytes_at(size));
            renderer.set_frames(device, queue, self.role, size, layers);
            if let Some(pass) = renderer.frames_mut(self.role) {
                pass.set_map(device, queue, self.map.as_ref());
            }
        }
        self.format = Some(format);
        self.playback = [Playback::default(); 2];
        self.modes = [None; 2];
    }

    /// Takes over a clip that finished loading, and makes frames passes for a camera
    /// that has opened (or reopened in another format). Returns the clip's path when
    /// one took over, or why it failed to load.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderers: [&mut Renderer; 2],
    ) -> Option<Result<PathBuf>> {
        if let Content::Camera(camera) = &self.content
            && let Some(format) = camera.format()
            && self.format != Some(format)
        {
            self.make_passes(device, queue, renderers, format);
            return None;
        }
        let result = self.loading.as_ref()?.poll()?;
        let path = self.loading.take().expect("a load finished").path.clone();
        match result {
            Ok(clip) => {
                self.info = describe_clip(&path, &clip);
                self.make_passes(device, queue, renderers, clip.format);
                self.content = Content::Clip(clip);
                Some(Ok(path))
            }
            Err(err) => Some(Err(err)),
        }
    }

    /// The preview starts showing something new: its clips continue from where the
    /// canvas shows them, as its clocks do.
    pub fn sync_preview(&mut self) {
        self.playback[View::Preview.index()] = self.playback[View::Main.index()];
    }

    /// Draws this canvas frame's picture into `renderer` (the canvas's or the
    /// preview's) for playback settings `v`. A paused camera holds its picture.
    #[allow(clippy::too_many_arguments)]
    pub fn feed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &mut Renderer,
        view: View,
        v: &VideoFrame,
        paused: bool,
    ) {
        if self.format.is_none() {
            return;
        }
        let Some(pass) = renderer.frames_mut(self.role) else {
            return;
        };
        let small = view == View::Preview;
        let mut sample = match &self.content {
            Content::Still => return,
            Content::Clip(clip) => {
                let i = view.index();
                if self.modes[i] != Some(v.mode) {
                    // Virtual indices mean other frames in another mode.
                    pass.forget();
                    self.modes[i] = Some(v.mode);
                }
                let count = clip.count();
                let index = self.playback[i].index(v, count, clip.fps);
                let sample = clip_sample(v, index, count, clip.fps, pass.max_layers());
                for k in pass.missing(device, &sample) {
                    let frame = &clip.frames[clip_frame(v.mode, k, count) as usize];
                    pass.upload(queue, k, if small { &frame.small } else { &frame.full });
                }
                sample
            }
            Content::Camera(camera) => {
                let arrivals = camera.arrivals();
                let Some(newest) = arrivals.last() else {
                    return;
                };
                self.held = paused.then(|| self.held.unwrap_or(newest.time));
                let shown = match self.held {
                    Some(time) => &arrivals[..arrivals.partition_point(|a| a.time <= time)],
                    None => &arrivals[..],
                };
                let Some(sample) = camera_sample(v, shown, pass.max_layers()) else {
                    return;
                };
                for k in pass.missing(device, &sample) {
                    if let Some(frame) = camera.frame(k) {
                        pass.upload(queue, k, if small { &frame.small } else { &frame.full });
                    }
                }
                sample
            }
        };
        if sample.slit == Slit::Map && self.map.is_none() {
            sample.slit = Slit::Rows;
        }
        renderer.draw_frames(device, queue, encoder, self.role, &sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_open_as_images_by_extension() {
        assert!(is_image(Path::new(r"C:\art\card.png")));
        assert!(is_image(Path::new("photo.JPG")));
        assert!(!is_image(Path::new("clip.mp4")));
        assert!(!is_image(Path::new("clip.mkv")));
        assert!(!is_image(Path::new("no-extension")));
    }

    #[test]
    fn frame_rates_read_naturally() {
        assert_eq!(describe_fps(24.0), "24");
        assert_eq!(describe_fps(1_800_000.0 / 75_001.0), "24");
        assert_eq!(describe_fps(30_000.0 / 1001.0), "29.97");
    }

    #[test]
    fn a_new_input_shows_an_image() {
        let input = Input::new(Role::Background);
        assert_eq!(input.kind(), Kind::Image);
        assert!(input.loading().is_none());
        assert!(input.camera_status().is_none());
    }
}
```

In `src/lib.rs`, replace:

```rust
pub mod gpu;
pub mod motion;
```

with:

```rust
pub mod gpu;
pub mod inputs;
pub mod motion;
```

In `src/passes/frames.rs`, replace:

```rust

    /// Uses `map` (or none) for slit-scan's Map.
```

with:

```rust

    /// The most layers the ring may grow to.
    pub fn max_layers(&self) -> u32 {
        self.max_layers
    }

    /// Forgets which frames the ring holds, for when virtual indices change meaning
    /// (a clip switching play mode).
    pub fn forget(&mut self) {
        self.held.fill(None);
    }

    /// Uses `map` (or none) for slit-scan's Map.
```

In `src/preview.rs`, replace:

```rust

    /// Forgets what was shown, so the trails are cleared when the preview appears again.
```

with:

```rust

    /// What the preview showed last, until it's hidden.
    pub fn shows(&self) -> Option<PreviewSource> {
        self.shown
    }

    /// Forgets what was shown, so the trails are cleared when the preview appears again.
```

- [ ] **Step 2: Run the new tests.** Run: `cargo test --lib inputs`. Expected: `3 passed`.

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `219 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `22 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 5: Manual check.** With `APPDATA` pointing at an empty scratch folder, run the app with a clip as its argument: `cargo run -- samples/flip.mp4` (any short video works). Within a second the canvas plays the clip through the deflection and colorizer, and the line under the header reads "Source: flip.mp4: 1280×736, 107 frames at 24 fps". Close the window and run `cargo run` again: the clip comes back from the autosave.

- [ ] **Step 6: Commit**

```bash
git add src/inputs.rs src/lib.rs src/app.rs src/passes/frames.rs src/preview.rs
git commit -m "feat: play clips and cameras as the source or background" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 9: The Source and Background sections

Each input gets a panel section: Image / Video / Camera, Open… or a camera list with its buffer length, what's showing (with loading progress and camera status), and the playback controls for the bank being edited.

**Files:**
- Create: `src/inputs_ui.rs`
- Modify: `src/lib.rs`, `src/ui.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `Kind`, `Input` (Task 8), `VideoParams` (Task 1), `CameraChoice` (Task 7), `camera::{list, Status}` (Task 4).
- Produces:
  - `inputs_ui::InputUi { kind, running, info, loading, status, note, camera, buffer_seconds, max_depth, map }` (default buffer 10 s), `inputs_ui::InputActions { open, use_camera, clear, choose_map, clear_map: Option<Role>, list_cameras: bool }`, `inputs_ui::input_section(ui, role, &mut InputUi, cameras: Option<&[String]>, &mut VideoParams, &mut InputActions)`.
  - `UiState.inputs: [InputUi; 2]`, `UiState.cameras: Option<Vec<String>>`, `UiActions.inputs: InputActions`; `UiState.source_info`, `UiState.background_info`, `UiActions.choose_background` and `UiActions.clear_background` are gone.

- [ ] **Step 1: Make the changes.**

In `src/app.rs`, replace:

```rust
use crate::inputs::{Input, View, is_image};
use crate::motion::{Mode, Motion};
```

with:

```rust
use crate::inputs::{Input, View, is_image};
use crate::inputs_ui::InputActions;
use crate::motion::{Mode, Motion};
```

In `src/app.rs`, replace:

```rust
use crate::session::{self, Autosave, InputFile, InputFiles, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};
```

with:

```rust
use crate::session::{self, Autosave, CameraChoice, InputFile, InputFiles, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};
use crate::video::camera::{self, Status};
```

In `src/app.rs`, replace:

```rust
        let image = test_card(&mut ui);
```

with:

```rust
        let image = test_card();
```

In `src/app.rs`, replace:

```rust
        };
        state.start_session(initial_image);
```

with:

```rust
        };
        state.feeds[Role::Source.index()].show_still(describe(TEST_CARD, &image));
        state.feeds[Role::Background.index()].show_still(NO_BACKGROUND.into());
        state.start_session(initial_image);
```

In `src/app.rs`, replace:

```rust
                    let info = format!("{}: {}", role.label(), feed.info);
                    match role {
                        Role::Source => self.ui.source_info = info,
                        Role::Background => self.ui.background_info = Some(info),
                    }
```

with:

```rust

```

In `src/app.rs`, replace:

```rust

    /// Shows the image at `path` as the source.
```

with:

```rust

    /// Shows each input's state in its panel section.
    fn show_inputs(&mut self) {
        for role in Role::ALL {
            let feed = &self.feeds[role.index()];
            let state = &mut self.ui.inputs[role.index()];
            let running = feed.kind();
            if state.running != running {
                // A file or camera took over: the panel follows.
                state.running = running;
                state.kind = running;
            }
            state.info = feed.info.clone();
            state.loading = feed
                .loading()
                .map(|(path, done)| format!("Loading {}… {:.0}%", file_name(path), done * 100.0));
            let status = feed.camera_status();
            state.status = status.as_ref().map(|status| {
                match status {
                    Status::Opening => "Opening the camera…",
                    Status::Live => "Live",
                    Status::NoSignal(_) => "No Signal",
                }
                .to_string()
            });
            state.note = match status {
                Some(Status::NoSignal(why)) => Some(why),
                _ => feed.camera_note(),
            };
            state.max_depth = feed.max_depth(&mut self.renderer);
            state.map = self.inputs.get(role).slit_map.as_deref().map(file_name);
        }
    }

    /// Carries out what the Source and Background sections asked for.
    fn input_actions(&mut self, actions: InputActions) {
        if actions.list_cameras {
            self.list_cameras();
        }
        if let Some(role) = actions.open {
            self.choose_file(role);
        }
        if let Some(role) = actions.use_camera {
            self.use_camera(role);
        }
        if let Some(role) = actions.clear {
            match role {
                Role::Source => self.show_test_card(),
                Role::Background => self.set_background(None),
            }
            self.remember_file(role, None);
        }
        if let Some(role) = actions.choose_map {
            self.choose_map(role);
        }
        if let Some(role) = actions.clear_map {
            let _ = self.open_map(role, None);
            let mut file = self.inputs.get(role);
            file.slit_map = None;
            self.inputs.set(role, file);
        }
    }

    fn list_cameras(&mut self) {
        let listed = camera::list();
        // Listing can take a moment; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
        self.ui.cameras = Some(listed.unwrap_or_else(|err| {
            log::warn!("{err:#}");
            self.ui.load_error = Some(format!("{err:#}"));
            Vec::new()
        }));
    }

    /// Asks for an image or video file for `role` and opens it.
    fn choose_file(&mut self, role: Role) {
        let picked = rfd::FileDialog::new()
            .set_title(format!("{} image or video", role.label()))
            .add_filter("Images and videos", MEDIA_EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.open_file(role, &path);
        }
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// Switches `role` to the camera picked in its section.
    fn use_camera(&mut self, role: Role) {
        let state = &self.ui.inputs[role.index()];
        let choice = CameraChoice {
            name: state.camera.clone(),
            buffer_seconds: state.buffer_seconds,
        };
        let canvas = self.renderer.size();
        self.feeds[role.index()].open_camera(&choice, canvas, &self.budget);
        let mut file = self.inputs.get(role);
        file.camera = Some(choice);
        self.inputs.set(role, file);
        self.ui.load_error = None;
    }

    /// Asks for a slit-scan map image for `role` and loads it.
    fn choose_map(&mut self, role: Role) {
        let picked = rfd::FileDialog::new()
            .set_title("Slit-scan map image")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            match self.open_map(role, Some(&path)) {
                Ok(()) => {
                    let mut file = self.inputs.get(role);
                    file.slit_map = Some(absolute(&path));
                    self.inputs.set(role, file);
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

    /// Shows the image at `path` as the source.
```

In `src/app.rs`, replace:

```rust
        self.ui.source_info = describe(&path.display().to_string(), &image);
        self.feeds[Role::Source.index()].show_still(self.ui.source_info.clone());
```

with:

```rust
        self.feeds[Role::Source.index()].show_still(describe(&file_name(path), &image));
```

In `src/app.rs`, replace:

```rust
        let image = test_card(&mut self.ui);
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
        self.feeds[Role::Source.index()].show_still(self.ui.source_info.clone());
    }

    /// Asks for a background image or video for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image or video")
            .add_filter("Images and videos", MEDIA_EXTENSIONS)
            .pick_file();
        if let Some(path) = picked {
            self.open_file(Role::Background, &path);
        }
        // The dialog and decoding block the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
```

with:

```rust
        let image = test_card();
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
        self.feeds[Role::Source.index()].show_still(describe(TEST_CARD, &image));
```

In `src/app.rs`, replace:

```rust
        self.ui.background_info = Some(format!(
            "Background: {} ({}×{})",
            path.display(),
            image.width,
            image.height
        ));
        Ok(())
    }

```

with:

```rust
        let info = format!("{} ({}×{})", file_name(path), image.width, image.height);
        self.feeds[Role::Background.index()].show_still(info);
        Ok(())
    }

    /// Shows `image` behind see-through levels, or black.
```

In `src/app.rs`, replace:

```rust
            self.ui.background_info = None;
        }
        self.feeds[Role::Background.index()].show_still(String::new());
```

with:

```rust
            self.feeds[Role::Background.index()].show_still(NO_BACKGROUND.into());
        }
```

In `src/app.rs`, replace:

```rust
            let feed = &mut self.feeds[role.index()];
            feed.open_camera(camera, canvas, &self.budget);
```

with:

```rust
            self.feeds[role.index()].open_camera(camera, canvas, &self.budget);
            let state = &mut self.ui.inputs[role.index()];
            state.camera = camera.name.clone();
            state.buffer_seconds = camera.buffer_seconds;
```

In `src/app.rs`, replace:

```rust
        self.poll_inputs();
        self.ui.late = self.clock.late();
```

with:

```rust
        self.poll_inputs();
        self.show_inputs();
        self.ui.late = self.clock.late();
```

In `src/app.rs`, replace:

```rust
        if actions.choose_background {
            self.choose_background();
        }
        if actions.clear_background {
            self.set_background(None);
            self.remember_file(Role::Background, None);
        }
```

with:

```rust
        self.input_actions(actions.inputs);
```

In `src/app.rs`, replace:

```rust
fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}
```

with:

```rust
/// What the source shows without a file or camera.
fn test_card() -> GrayImage {
    source::test_card(1600, 900)
}

const TEST_CARD: &str = "Built-in test card";
const NO_BACKGROUND: &str = "No background: see-through levels show black.";
```

In `src/app.rs`, replace:

```rust
    format!("Source: {name} ({}×{})", image.width, image.height)
```

with:

```rust
    format!("{name} ({}×{})", image.width, image.height)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
```

Create `src/inputs_ui.rs`:

```rust
//! The panel's Source and Background sections: what each input is (an image, a video
//! or a camera) and how a video or camera plays. They only report what was asked for;
//! the app opens files and cameras.

use egui::{CollapsingHeader, ComboBox, Slider, Ui};

use crate::inputs::Kind;
use crate::params::{Between, PlayMode, Role, Slit, VideoParams, ranges};
use crate::video::camera::{BUFFER_SECONDS, DEFAULT_BUFFER_SECONDS};

/// What an input's section shows, kept between frames.
#[derive(Clone, Debug)]
pub struct InputUi {
    /// The kind picked in the panel; it takes over once a file or camera is chosen.
    pub kind: Kind,
    /// The kind actually showing.
    pub running: Kind,
    /// What's showing.
    pub info: String,
    /// While a clip loads: "Loading flip.mp4… 40%".
    pub loading: Option<String>,
    /// A camera's state: "Opening…", "Live" or "No Signal".
    pub status: Option<String>,
    /// Why a camera has no signal, or that its buffer had to be shortened.
    pub note: Option<String>,
    /// The camera picked in the list.
    pub camera: String,
    /// Seconds of frames the camera keeps.
    pub buffer_seconds: f32,
    /// The deepest slit-scan the GPU's frame ring holds, in seconds.
    pub max_depth: Option<f32>,
    /// The slit-scan map image's file name, if one is loaded.
    pub map: Option<String>,
}

impl Default for InputUi {
    fn default() -> Self {
        Self {
            kind: Kind::Image,
            running: Kind::Image,
            info: String::new(),
            loading: None,
            status: None,
            note: None,
            camera: String::new(),
            buffer_seconds: DEFAULT_BUFFER_SECONDS,
            max_depth: None,
            map: None,
        }
    }
}

/// One-shot requests from the input sections this frame.
#[derive(Default)]
pub struct InputActions {
    /// Ask for an image or video file.
    pub open: Option<Role>,
    /// Switch to the camera picked in the list.
    pub use_camera: Option<Role>,
    /// Show nothing: the test card, or a black background.
    pub clear: Option<Role>,
    pub choose_map: Option<Role>,
    pub clear_map: Option<Role>,
    /// List the cameras again.
    pub list_cameras: bool,
}

/// The Source or Background section. `cameras` is the listed devices (none until
/// listed); `video` is the edited bank's playback settings for this input.
pub fn input_section(
    ui: &mut Ui,
    role: Role,
    state: &mut InputUi,
    cameras: Option<&[String]>,
    video: &mut VideoParams,
    actions: &mut InputActions,
) {
    CollapsingHeader::new(role.label())
        .default_open(role == Role::Source)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut state.kind, kind, kind.label());
                }
            });
            match state.kind {
                Kind::Image | Kind::Video => {
                    ui.horizontal(|ui| {
                        if ui.button("Open…").clicked() {
                            actions.open = Some(role);
                        }
                        let nothing = match role {
                            Role::Source => "Test card",
                            Role::Background => "No background",
                        };
                        if ui.button(nothing).clicked() {
                            actions.clear = Some(role);
                        }
                    });
                }
                Kind::Camera => camera_choice(ui, role, state, cameras, actions),
            }
            ui.small(&state.info);
            for line in [&state.loading, &state.status, &state.note]
                .into_iter()
                .flatten()
            {
                ui.small(line);
            }
            if state.running != Kind::Image {
                playback(ui, role, state, video, actions);
            }
        });
}

fn camera_choice(
    ui: &mut Ui,
    role: Role,
    state: &mut InputUi,
    cameras: Option<&[String]>,
    actions: &mut InputActions,
) {
    let Some(cameras) = cameras else {
        actions.list_cameras = true;
        ui.small("Looking for cameras…");
        return;
    };
    ui.horizontal(|ui| {
        let shown = if state.camera.is_empty() {
            "Choose a camera"
        } else {
            &state.camera
        };
        ComboBox::from_id_salt(("camera", role.index()))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for name in cameras {
                    ui.selectable_value(&mut state.camera, name.clone(), name);
                }
            });
        if ui.button("Refresh").clicked() {
            actions.list_cameras = true;
        }
    });
    if cameras.is_empty() {
        ui.small("No cameras found.");
    }
    ui.add(Slider::new(&mut state.buffer_seconds, BUFFER_SECONDS).text("buffer (s)"))
        .on_hover_text("Seconds of frames kept for delay and slit-scan");
    let label = if state.running == Kind::Camera {
        "Restart camera"
    } else {
        "Use camera"
    };
    if ui
        .add_enabled(!state.camera.is_empty(), egui::Button::new(label))
        .clicked()
    {
        actions.use_camera = Some(role);
    }
}

/// How a video or camera plays: it edits the same bank or cue as the rest of the panel.
fn playback(
    ui: &mut Ui,
    role: Role,
    state: &InputUi,
    v: &mut VideoParams,
    actions: &mut InputActions,
) {
    ui.separator();
    if state.running == Kind::Video {
        ui.horizontal(|ui| {
            for mode in PlayMode::ALL {
                ui.selectable_value(&mut v.mode, mode, mode.label());
            }
        });
        if v.mode == PlayMode::Scrub {
            ui.add(Slider::new(&mut v.position, ranges::POSITION).text("position"));
        } else {
            ui.add(Slider::new(&mut v.speed, ranges::PLAY_SPEED).text("speed"))
                .on_hover_text("× the clip's own frame rate; negative plays backwards");
        }
    } else {
        // Shown up to the buffer length; a longer delay from a file shows the oldest
        // frame. Only an edit changes the value, so drawing it isn't a change.
        let buffer = state.buffer_seconds.min(*ranges::DELAY.end());
        let mut delay = v.delay.min(buffer);
        if ui
            .add(Slider::new(&mut delay, 0.0..=buffer).text("delay (s)"))
            .changed()
        {
            v.delay = delay;
        }
    }
    ui.horizontal(|ui| {
        ui.label("Between frames");
        for between in Between::ALL {
            ui.selectable_value(&mut v.between, between, between.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Slit-scan");
        for slit in Slit::ALL {
            ui.selectable_value(&mut v.slit, slit, slit.label());
        }
    });
    if v.slit == Slit::Off {
        return;
    }
    let deepest = state
        .max_depth
        .unwrap_or(*ranges::SLIT_DEPTH.end())
        .min(*ranges::SLIT_DEPTH.end());
    // Shown up to the deepest the ring allows; a deeper value from a file is used at
    // that depth. Only an edit changes the value.
    let mut depth = v.slit_depth.min(deepest);
    if ui
        .add(Slider::new(&mut depth, 0.0..=deepest).text("depth (s)"))
        .on_hover_text(format!("Up to {deepest:.1} s fits in the GPU's frame ring"))
        .changed()
    {
        v.slit_depth = depth;
    }
    ui.checkbox(&mut v.slit_flip, "Flip");
    if v.slit == Slit::Map {
        ui.horizontal(|ui| {
            if ui.button("Map image…").clicked() {
                actions.choose_map = Some(role);
            }
            if state.map.is_some() && ui.button("Clear").clicked() {
                actions.clear_map = Some(role);
            }
        });
        ui.small(
            state
                .map
                .as_deref()
                .unwrap_or("No map image: Map works as Rows."),
        );
    }
}
```

In `src/lib.rs`, replace:

```rust
pub mod inputs;
pub mod motion;
```

with:

```rust
pub mod inputs;
pub mod inputs_ui;
pub mod motion;
```

In `src/ui.rs`, replace:

```rust
use crate::motion::{Mode, Motion};
use crate::params::{
    Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, even_thresholds, ranges,
```

with:

```rust
use crate::inputs_ui::{self, InputActions, InputUi};
use crate::motion::{Mode, Motion};
use crate::params::{
    Axis, Envelope, OscInput, OscSync, Oscillator, Params, Role, Waveform, even_thresholds, ranges,
```

In `src/ui.rs`, replace:

```rust
    pub source_info: String,
```

with:

```rust

```

In `src/ui.rs`, replace:

```rust
    /// The keying background's description, if one is loaded.
    pub background_info: Option<String>,
```

with:

```rust
    /// The Source and Background sections, indexed by [`Role::index`].
    pub inputs: [InputUi; 2],
    /// The cameras DirectShow listed; none until they're first needed.
    pub cameras: Option<Vec<String>>,
```

In `src/ui.rs`, replace:

```rust
    pub choose_background: bool,
    pub clear_background: bool,
    pub files: FileActions,
```

with:

```rust
    pub files: FileActions,
    pub inputs: InputActions,
```

In `src/ui.rs`, replace:

```rust
                ui.label(&state.source_info);
```

with:

```rust

```

In `src/ui.rs`, replace:

```rust
                ui.label("Drop an image, preset or project onto the window.");
```

with:

```rust
                ui.label("Drop an image, video, preset or project onto the window.");
```

In `src/ui.rs`, replace:

```rust
                mode_section(ui, motion);
                files_ui::presets_section(
```

with:

```rust
                mode_section(ui, motion);
                for role in Role::ALL {
                    inputs_ui::input_section(
                        ui,
                        role,
                        &mut state.inputs[role.index()],
                        state.cameras.as_deref(),
                        motion.editable().video_mut(role),
                        &mut actions.inputs,
                    );
                }
                files_ui::presets_section(
```

In `src/ui.rs`, replace:

```rust
                key_section(ui, params, state, &mut actions);
```

with:

```rust
                key_section(ui, params);
```

In `src/ui.rs`, replace:

```rust
fn key_section(ui: &mut Ui, params: &mut Params, state: &UiState, actions: &mut UiActions) {
```

with:

```rust
fn key_section(ui: &mut Ui, params: &mut Params) {
```

In `src/ui.rs`, replace:

```rust
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
```

with:

```rust
        ui.small("See-through levels show the Background input.");
```

- [ ] **Step 2: Run everything.** Run: `cargo test`. Expected: `219 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `22 passed` in `tests/smoke.rs` and `6 passed` in `tests/video.rs`.

- [ ] **Step 3: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 4: Manual check.** With `APPDATA` pointing at a scratch folder, run `cargo run -- samples/flip.mp4`. The Source section shows **Video** selected, "flip.mp4: 1280×736, 107 frames at 24 fps", Loop / Ping-pong / Scrub with a speed slider, Between frames, and Slit-scan; choosing **Rows** shows a depth slider (1.0) and Flip, and the picture smears through time from top to bottom. In Background, choose **Camera**: the list fills with the connected cameras; pick one and press **Use camera**, and the section reads "Live". Save the project, then expand and collapse both sections: the header shows no unsaved-changes dot, because drawing the controls changes nothing.

- [ ] **Step 5: Commit**

```bash
git add src/inputs_ui.rs src/lib.rs src/ui.rs src/app.rs
git commit -m "feat: Source and Background panel sections" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
