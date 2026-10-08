# Rasterwarp Program Frame Rate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the whole canvas at one chosen frame rate (23.976, 24, 25, 29.97, 30, 50, 59.94 or 60 fps), so motion and trails on screen match recordings exactly and recordings never judder, on top of Plan 2 (Output) now on `main`.

**Architecture:**
- **Rate:** `FrameRate` holds the rate as an exact fraction. `CanvasClock` decides, on each screen refresh, how many canvas frames are due at that rate.
  - Up to 4 due frames are drawn back to back (catch-up).
  - Frames further behind are skipped without advancing animation time.
  - Offline recording draws exactly one frame per refresh.
- **Renderer:** `Renderer::render` splits into `render_canvas` (advance the canvas by one frame) and `composite` (show the current canvas), so refreshes with no frame due only re-composite and redraw the UI.
- **Recording:** recordings take every canvas frame as the next frame of the file. The encoder writes the exact rate fraction.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9 (shared FFmpeg 9.0.2), chrono 0.4, rfd 0.17.

**Spec:** `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md`, section "Frame rate (added after Plan 2)", plus the Capture section's timing and offline bullets.

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- No new crates. The versions stay pinned: wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9.0.0, chrono 0.4, rfd 0.17.
- FFmpeg and libclang paths are already in `.cargo/config.toml`. `build.rs` copies the FFmpeg DLLs next to the executables.
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Don't use the word "Scanimate" in user-facing text.
- Rates, exactly: 24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001, 60 (default). Up to 4 canvas frames per refresh (`MAX_CATCH_UP`).
- Code in this plan was compiled, tested and run on the dev machine before the plan was written. Every task's end state was also built and tested on its own. Transcribe it exactly.

## Design decisions made while building (beyond the spec text)

1. **A canvas frame is due once the wall-clock time is within half a period of it.** Display timestamps jitter by a few hundred microseconds. A frame due exactly on a refresh would otherwise flip between two refreshes, and 60 fps on a 60 Hz display would alternate 0 and 2 frames per refresh. This is the same reasoning as the earlier nearest-frame fix to real-time capture.
2. **Skipped late frames never fall due and don't advance animation time.** After a long stall, animation simply carries on, and recordings have no gap.
   - Stalls the app causes itself re-anchor the clock instead of counting as late: startup, opening the encoder, finishing a file, and the folder dialog.
3. **Pause keeps drawing canvas frames at the rate, with animation time frozen.** Parameter edits stay visible while paused, as before. Paused frames are not recorded.
4. **Each canvas frame is submitted on its own**, so its readback starts mapping before the next catch-up frame. When all 3 staging buffers are still busy, real-time capture now **waits for the GPU** instead of dropping the frame. Only a full encoder queue drops frames.
5. **Recordings no longer pick frames from the display rate.** `FrameClock` just numbers the captured frames and converts them to seconds at the recording's rate. `OFFLINE_STEP` and `encode::FPS` are gone.
6. **Offline pacing anchors the real-time clock at every offline frame**, so stopping an offline recording carries on without a jump or a burst of catch-up frames.
7. **The capture panel's "below 60 fps" warning is replaced** by one that appears when the encoder drops frames, which is now the only way a real-time recording loses frames. The header shows the canvas rate, the display's refresh rate and the late count.
8. **Verified in the real app on a 60 Hz display:**
   - 24 fps real-time HEVC: 112 frames over 4.7 s, declared `24/1`, every pts exactly one frame apart, 0 late.
   - 23.976 fps offline FFV1 with stop-after 2 s: 48 frames, declared `24000/1001`.

## File Structure

| File | Responsibility |
|---|---|
| `src/rate.rs` (new) | `FrameRate`: the exact fraction, labels, periods, frame/second conversions |
| `src/clock.rs` (new) | `CanvasClock` and `Pacing`: how many canvas frames each refresh draws |
| `src/capture/encode.rs` (modify) | `Encoder::start` takes the `FrameRate`; time base, declared rate and keyframe interval follow it |
| `src/capture/mod.rs` (modify) | `FrameClock` numbers captured frames; no more display-rate decimation |
| `src/capture/recorder.rs` (modify) | `Recorder::start` takes the rate; `capture` records every call; waits for the GPU when the ring is full |
| `src/passes/mod.rs` (modify) | `Renderer::render_canvas` and `Renderer::composite` |
| `src/app.rs` (modify) | The refresh loop driven by `CanvasClock`; one submit per canvas frame |
| `src/ui.rs` (modify) | The Frame rate menu, the header readout, the encoder-drop warning |

---

### Task 1: Frame rates and the canvas clock

**Files:**
- Create: `src/rate.rs`, `src/clock.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/rate.rs` and `src/clock.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `rate::FrameRate { pub num: i32, pub den: i32 }` (`Copy`, `Eq`, `Default` = 60 fps):
    - `ALL: [FrameRate; 8]` (slowest first), `whole(fps)`, `ntsc(fps)` (`fps × 1000/1001`);
    - `fps() -> f64`, `period() -> f64` (seconds), `seconds(frames: i64) -> f64`, `frames_in(seconds: f64) -> i64` (rounded);
    - `keyframe_interval() -> u32`, `label() -> String` ("23.976 fps", "24 fps", "29.97 fps").
  - `clock::MAX_CATCH_UP` (4) and `clock::Pacing { RealTime, Offline }`.
  - `clock::CanvasClock`:
    - `new(rate, now)`, `rate()`, `late() -> u64`;
    - `reanchor(now)`: the next frame is due at `now`;
    - `set_rate(rate, now)`;
    - `due(now, Pacing) -> u32`: canvas frames to draw on this refresh.
    - `now` is wall-clock seconds as `f64`, from any fixed origin.

- [ ] **Step 1: Register the modules.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod canvas;
pub mod capture;
pub mod clock;
pub mod curve;
pub mod curve_editor;
pub mod gpu;
pub mod motion;
pub mod params;
pub mod passes;
pub mod preview;
pub mod rate;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing frame-rate tests.** Create `src/rate.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_rates_are_exact_fractions() {
        assert_eq!(
            FrameRate::ntsc(24),
            FrameRate {
                num: 24000,
                den: 1001
            }
        );
        assert!((FrameRate::ntsc(30).fps() - 29.97003).abs() < 1e-5);
        assert!((FrameRate::whole(25).period() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn labels_drop_trailing_zeros() {
        let labels: Vec<String> = FrameRate::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(
            labels,
            [
                "23.976 fps",
                "24 fps",
                "25 fps",
                "29.97 fps",
                "30 fps",
                "50 fps",
                "59.94 fps",
                "60 fps"
            ]
        );
    }

    #[test]
    fn frames_and_seconds_round_trip() {
        let rate = FrameRate::ntsc(24);
        assert_eq!(rate.frames_in(rate.seconds(1000)), 1000);
        assert_eq!(FrameRate::whole(60).frames_in(10.0 / 60.0), 10);
        assert_eq!(rate.keyframe_interval(), 24);
        assert_eq!(FrameRate::default(), FrameRate::whole(60));
    }
}
```

- [ ] **Step 3: Write the failing clock tests.** Create `src/clock.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Refresh times of a 60 Hz display with ±0.3 ms of jitter.
    fn refresh(i: u32) -> f64 {
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        f64::from(i) / 60.0 + jitter[i as usize % 5]
    }

    #[test]
    fn sixty_fps_on_a_60_hz_display_draws_one_frame_per_refresh() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        let counts: Vec<u32> = (0..600)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert!(counts.iter().all(|&n| n == 1), "{counts:?}");
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn twenty_four_fps_on_a_60_hz_display_alternates_two_and_three_refreshes() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        let counts: Vec<u32> = (0..10)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert_eq!(counts, [1, 0, 1, 0, 1, 0, 0, 1, 0, 1]);
    }

    #[test]
    fn ntsc_rate_does_not_drift_over_an_hour() {
        let rate = FrameRate::ntsc(24);
        let mut clock = CanvasClock::new(rate, 0.0);
        let refreshes = 60 * 3600;
        let mut drawn = 0u64;
        for i in 0..refreshes {
            let n = clock.due(refresh(i), Pacing::RealTime);
            assert!(n <= 1, "refresh {i} drew {n} frames");
            drawn += u64::from(n);
        }
        let expected = rate.frames_in(refresh(refreshes - 1)) as u64 + 1;
        assert_eq!(drawn, expected);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn catches_up_after_a_short_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(3.0 / 60.0, Pacing::RealTime), 3);
        assert_eq!(clock.due(4.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn a_long_stall_draws_four_frames_and_skips_the_rest() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(10.0 / 60.0, Pacing::RealTime), MAX_CATCH_UP);
        assert_eq!(clock.late(), 6);
        assert_eq!(
            clock.due(11.0 / 60.0, Pacing::RealTime),
            1,
            "no longer behind"
        );
    }

    #[test]
    fn offline_draws_one_frame_per_refresh_and_hands_back_without_a_jump() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        for i in 1..=5 {
            assert_eq!(clock.due(f64::from(i) * 0.5, Pacing::Offline), 1);
        }
        assert_eq!(clock.due(2.5 + 1.0 / 60.0, Pacing::RealTime), 0);
        assert_eq!(clock.due(2.5 + 1.0 / 24.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn reanchoring_forgives_a_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.reanchor(5.0);
        assert_eq!(clock.due(5.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(5.0 + 1.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn changing_the_rate_paces_from_the_change() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.set_rate(FrameRate::whole(25), 1.0);
        assert_eq!(clock.rate(), FrameRate::whole(25));
        assert_eq!(clock.due(1.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(1.01, Pacing::RealTime), 0);
        assert_eq!(clock.due(1.04, Pacing::RealTime), 1);
    }
}
```

- [ ] **Step 4: Run them and confirm they fail.** Run: `cargo test --lib -- rate:: clock::`. Expected: compile errors such as "cannot find type `FrameRate`" and "cannot find type `CanvasClock`".

- [ ] **Step 5: Implement `FrameRate`.** Replace `src/rate.rs` with:

```rust
//! The program frame rate: every canvas frame advances animation by exactly one period,
//! and recordings run at the same rate.

/// A frame rate as an exact fraction, `num / den` frames per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRate {
    pub num: i32,
    pub den: i32,
}

impl FrameRate {
    /// The rates the panel offers, slowest first.
    pub const ALL: [FrameRate; 8] = [
        FrameRate::ntsc(24),
        FrameRate::whole(24),
        FrameRate::whole(25),
        FrameRate::ntsc(30),
        FrameRate::whole(30),
        FrameRate::whole(50),
        FrameRate::ntsc(60),
        FrameRate::whole(60),
    ];

    /// `fps` frames per second exactly.
    pub const fn whole(fps: i32) -> Self {
        Self { num: fps, den: 1 }
    }

    /// The NTSC rate just below `fps`: `fps × 1000/1001` (23.976, 29.97, 59.94).
    pub const fn ntsc(fps: i32) -> Self {
        Self {
            num: fps * 1000,
            den: 1001,
        }
    }

    /// Frames per second.
    pub fn fps(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Seconds per frame.
    pub fn period(self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    /// Seconds of `frames` frames.
    pub fn seconds(self, frames: i64) -> f64 {
        frames as f64 * self.period()
    }

    /// Whole frames in `seconds`, rounded to the nearest frame.
    pub fn frames_in(self, seconds: f64) -> i64 {
        (seconds * self.fps()).round() as i64
    }

    /// Frames between keyframes in a recording: about one second.
    pub fn keyframe_interval(self) -> u32 {
        self.fps().round() as u32
    }

    /// "24 fps", "23.976 fps", "29.97 fps".
    pub fn label(self) -> String {
        let fps = format!("{:.3}", self.fps());
        let fps = fps.trim_end_matches('0').trim_end_matches('.');
        format!("{fps} fps")
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        FrameRate::whole(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_rates_are_exact_fractions() {
        assert_eq!(
            FrameRate::ntsc(24),
            FrameRate {
                num: 24000,
                den: 1001
            }
        );
        assert!((FrameRate::ntsc(30).fps() - 29.97003).abs() < 1e-5);
        assert!((FrameRate::whole(25).period() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn labels_drop_trailing_zeros() {
        let labels: Vec<String> = FrameRate::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(
            labels,
            [
                "23.976 fps",
                "24 fps",
                "25 fps",
                "29.97 fps",
                "30 fps",
                "50 fps",
                "59.94 fps",
                "60 fps"
            ]
        );
    }

    #[test]
    fn frames_and_seconds_round_trip() {
        let rate = FrameRate::ntsc(24);
        assert_eq!(rate.frames_in(rate.seconds(1000)), 1000);
        assert_eq!(FrameRate::whole(60).frames_in(10.0 / 60.0), 10);
        assert_eq!(rate.keyframe_interval(), 24);
        assert_eq!(FrameRate::default(), FrameRate::whole(60));
    }
}
```

- [ ] **Step 6: Implement `CanvasClock`.** Replace `src/clock.rs` with:

```rust
//! Paces canvas frames at the program frame rate, independently of the display's
//! refresh rate.

use crate::rate::FrameRate;

/// Most canvas frames drawn on one screen refresh when the app has fallen behind.
pub const MAX_CATCH_UP: u32 = 4;

/// How canvas frames are paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// Follow wall-clock time, catching up on late frames.
    RealTime,
    /// One frame per refresh, whatever the time (offline recording).
    Offline,
}

/// Decides how many canvas frames each screen refresh draws. Frame `anchor_frame` is due
/// at wall-clock time `anchor`, and frame `n` one period per frame after it.
#[derive(Clone, Debug)]
pub struct CanvasClock {
    rate: FrameRate,
    anchor: f64,
    anchor_frame: i64,
    /// Canvas frames drawn so far; also the index of the next one.
    drawn: i64,
    /// Frames that were due but skipped because the app fell too far behind.
    late: u64,
}

impl CanvasClock {
    /// A clock whose first frame is due at wall-clock time `now` (in seconds).
    pub fn new(rate: FrameRate, now: f64) -> Self {
        Self {
            rate,
            anchor: now,
            anchor_frame: 0,
            drawn: 0,
            late: 0,
        }
    }

    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    /// Frames skipped so far because the app fell more than [`MAX_CATCH_UP`] behind.
    pub fn late(&self) -> u64 {
        self.late
    }

    /// Makes the next frame due at `now`, as after a stall that shouldn't count as late
    /// (opening an encoder, a dialog).
    pub fn reanchor(&mut self, now: f64) {
        self.anchor = now;
        self.anchor_frame = self.drawn;
    }

    /// Switches to `rate`; the next frame is due at `now`.
    pub fn set_rate(&mut self, rate: FrameRate, now: f64) {
        self.rate = rate;
        self.reanchor(now);
    }

    /// How many canvas frames to draw on the refresh at `now`. Real-time pacing draws the
    /// frames that are due (a frame is due once `now` is within half a period of its time,
    /// so a display's jitter doesn't move frames between refreshes), up to
    /// [`MAX_CATCH_UP`]; frames further behind are skipped without ever falling due.
    /// Offline pacing always draws one frame and keeps real-time pacing in step with it.
    pub fn due(&mut self, now: f64, pacing: Pacing) -> u32 {
        let count = match pacing {
            Pacing::Offline => {
                // The frame drawn now counts as due now.
                self.anchor = now;
                self.anchor_frame = self.drawn;
                1
            }
            Pacing::RealTime => {
                let newest = self.anchor_frame
                    + ((now - self.anchor) * self.rate.fps() + 0.5).floor() as i64;
                let behind = (newest + 1 - self.drawn).max(0);
                let skipped = (behind - i64::from(MAX_CATCH_UP)).max(0);
                self.late += skipped as u64;
                self.anchor_frame -= skipped;
                (behind - skipped) as u32
            }
        };
        self.drawn += i64::from(count);
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refresh times of a 60 Hz display with ±0.3 ms of jitter.
    fn refresh(i: u32) -> f64 {
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        f64::from(i) / 60.0 + jitter[i as usize % 5]
    }

    #[test]
    fn sixty_fps_on_a_60_hz_display_draws_one_frame_per_refresh() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        let counts: Vec<u32> = (0..600)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert!(counts.iter().all(|&n| n == 1), "{counts:?}");
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn twenty_four_fps_on_a_60_hz_display_alternates_two_and_three_refreshes() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        let counts: Vec<u32> = (0..10)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert_eq!(counts, [1, 0, 1, 0, 1, 0, 0, 1, 0, 1]);
    }

    #[test]
    fn ntsc_rate_does_not_drift_over_an_hour() {
        let rate = FrameRate::ntsc(24);
        let mut clock = CanvasClock::new(rate, 0.0);
        let refreshes = 60 * 3600;
        let mut drawn = 0u64;
        for i in 0..refreshes {
            let n = clock.due(refresh(i), Pacing::RealTime);
            assert!(n <= 1, "refresh {i} drew {n} frames");
            drawn += u64::from(n);
        }
        let expected = rate.frames_in(refresh(refreshes - 1)) as u64 + 1;
        assert_eq!(drawn, expected);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn catches_up_after_a_short_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(3.0 / 60.0, Pacing::RealTime), 3);
        assert_eq!(clock.due(4.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn a_long_stall_draws_four_frames_and_skips_the_rest() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(10.0 / 60.0, Pacing::RealTime), MAX_CATCH_UP);
        assert_eq!(clock.late(), 6);
        assert_eq!(
            clock.due(11.0 / 60.0, Pacing::RealTime),
            1,
            "no longer behind"
        );
    }

    #[test]
    fn offline_draws_one_frame_per_refresh_and_hands_back_without_a_jump() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        for i in 1..=5 {
            assert_eq!(clock.due(f64::from(i) * 0.5, Pacing::Offline), 1);
        }
        assert_eq!(clock.due(2.5 + 1.0 / 60.0, Pacing::RealTime), 0);
        assert_eq!(clock.due(2.5 + 1.0 / 24.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn reanchoring_forgives_a_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.reanchor(5.0);
        assert_eq!(clock.due(5.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(5.0 + 1.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn changing_the_rate_paces_from_the_change() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.set_rate(FrameRate::whole(25), 1.0);
        assert_eq!(clock.rate(), FrameRate::whole(25));
        assert_eq!(clock.due(1.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(1.01, Pacing::RealTime), 0);
        assert_eq!(clock.due(1.04, Pacing::RealTime), 1);
    }
}
```

- [ ] **Step 7: Run the tests and confirm they pass.** Run: `cargo test --lib -- rate:: clock::`. Expected: `11 passed`. Then `cargo test`: `122 passed` for the unit tests, `3 passed` in `tests/capture.rs`, `6 passed` in `tests/smoke.rs`.

- [ ] **Step 8: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs src/rate.rs src/clock.rs
git commit -m "feat: add frame rates and a canvas clock" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Record at the program frame rate

**Files:**
- Modify: `src/capture/encode.rs`, `src/capture/mod.rs`, `src/capture/recorder.rs`, `src/app.rs`, `tests/capture.rs`, `tests/smoke.rs`
- Test: `tests/capture.rs` (adds `recordings_declare_an_ntsc_rate_exactly`); `FrameClock` unit tests; `tests/smoke.rs` (offline at 23.976 fps, and real-time waiting for a full ring)

**Interfaces:**
- Consumes: `FrameRate` (Task 1).
- Produces:
  - `Encoder::start(path, format, size, rate: FrameRate)`. `encode::FPS` is removed.
  - `capture::FrameClock`:
    - `new(rate)`, `next_pts() -> i64`, `frames()`, `seconds()`;
    - `reached(stop_after: f32)`, which counts whole frames at the rate.
    - `capture::OFFLINE_STEP` is removed.
  - `Recorder::start(device, &RecordSettings, canvas, rate: FrameRate)`.
  - `Recorder::capture(device, encoder, draw)`:
    - It has no `time` argument. Every call records the next frame.
    - When every staging buffer is busy it waits for the GPU.
    - The caller submits each frame and calls `after_submit` before the next `capture`.
  - `RecordStatus.dropped` now counts only frames the encoder's queue couldn't take.
  - For this task the app records with `FrameRate::default()` (60 fps) and keeps its old timing. Task 4 replaces both.

> The ffmpeg-next `Rational` type has `invert()`, which gives the one-frame time base from the frame rate.

- [ ] **Step 1: Write the failing encoder tests.** Replace `tests/capture.rs` with the following. It passes a rate to `encode` and `decode`, checks the declared rate exactly, and adds an NTSC-rate file.

```rust
//! Encodes short clips with the linked FFmpeg libraries and decodes them again.

use std::path::{Path, PathBuf};

use ffmpeg_next as ff;
use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};
use rasterwarp::rate::FrameRate;

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 30;

/// A fresh path in a per-test temporary directory.
fn temp_file(test: &str, extension: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(format!("clip.{extension}"))
}

/// A moving gradient with some fine detail, different in every frame.
fn pattern(index: i64) -> Vec<u8> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let shift = index as usize * 4;
            rgba.extend_from_slice(&[
                ((x + shift) * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x / 8 + y / 8) % 2 * 64 + shift) as u8,
                255,
            ]);
        }
    }
    rgba
}

fn encode(path: &Path, format: VideoFormat, rate: FrameRate) -> anyhow::Result<u64> {
    let encoder = Encoder::start(path, format, SIZE, rate)?;
    for pts in 0..FRAMES {
        encoder.send(Frame {
            pts,
            rgba: pattern(pts),
        })?;
    }
    encoder.finish()
}

/// Decodes every frame of `path` to tightly packed RGBA (BT.709 for YUV input), checking
/// that the file declares `rate`.
fn decode(path: &Path, rate: FrameRate) -> Vec<(u32, u32, Vec<u8>)> {
    ff::init().unwrap();
    let mut input = ff::format::input(path).expect("open the recording");
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .expect("a video stream");
    let index = stream.index();
    let declared = ff::Rational(rate.num, rate.den);
    assert_eq!(stream.avg_frame_rate(), declared, "declared frame rate");
    assert_eq!(stream.rate(), declared, "stream frame rate");
    let context = ff::codec::context::Context::from_parameters(stream.parameters()).unwrap();
    let mut decoder = context.decoder().video().unwrap();
    let (w, h) = (decoder.width(), decoder.height());
    let mut scaler = ff::software::scaling::Context::get(
        decoder.format(),
        w,
        h,
        ff::format::Pixel::RGBA,
        w,
        h,
        ff::software::scaling::Flags::BILINEAR,
    )
    .unwrap();
    if decoder.format() == ff::format::Pixel::YUV444P {
        // SAFETY: valid scaler pointer; FFmpeg's coefficient tables are static.
        unsafe {
            let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
            ff::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                bt709,
                0,
                bt709,
                1,
                0,
                1 << 16,
                1 << 16,
            );
        }
    }
    let mut frames = Vec::new();
    let mut receive = |decoder: &mut ff::decoder::Video, frames: &mut Vec<_>| {
        let mut decoded = ff::frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba = ff::frame::Video::empty();
            scaler.run(&decoded, &mut rgba).unwrap();
            let row = w as usize * 4;
            let packed: Vec<u8> = rgba
                .data(0)
                .chunks(rgba.stride(0))
                .take(h as usize)
                .flat_map(|r| &r[..row])
                .copied()
                .collect();
            frames.push((w, h, packed));
        }
    };
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            decoder.send_packet(&packet).unwrap();
            receive(&mut decoder, &mut frames);
        }
    }
    decoder.send_eof().unwrap();
    receive(&mut decoder, &mut frames);
    frames
}

#[test]
fn ffv1_recording_is_lossless() {
    let path = temp_file("ffv1", "mkv");
    let rate = FrameRate::default();
    assert_eq!(encode(&path, VideoFormat::Ffv1, rate).expect("encode"), 30);
    let frames = decode(&path, rate);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    assert!(
        *rgba == pattern(7),
        "frame 7 differs from what was recorded"
    );
}

#[test]
fn hevc_recording_is_close_to_the_source() {
    let path = temp_file("hevc", "mp4");
    let rate = FrameRate::default();
    let written = match encode(&path, VideoFormat::Hevc, rate) {
        Ok(written) => written,
        Err(err) => {
            eprintln!("skipping HEVC test: {err:#}");
            return;
        }
    };
    assert_eq!(written, 30);
    let frames = decode(&path, rate);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    let source = pattern(7);
    let error: u64 = rgba
        .iter()
        .zip(&source)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = error as f64 / source.len() as f64;
    assert!(mean < 3.0, "mean error {mean:.2} per channel");
}

#[test]
fn recordings_declare_an_ntsc_rate_exactly() {
    let path = temp_file("ntsc", "mkv");
    let rate = FrameRate::ntsc(24);
    assert_eq!(encode(&path, VideoFormat::Ffv1, rate).expect("encode"), 30);
    assert_eq!(decode(&path, rate).len(), FRAMES as usize);
}

#[test]
fn an_existing_file_is_not_overwritten() {
    let path = temp_file("exists", "mkv");
    std::fs::write(&path, b"keep me").unwrap();
    let err = Encoder::start(&path, VideoFormat::Ffv1, SIZE, FrameRate::default())
        .err()
        .expect("start must fail");
    assert!(format!("{err:#}").contains("already exists"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}
```

- [ ] **Step 2: Run them and confirm they fail.** Run: `cargo test --test capture`. Expected: compile errors such as "this function takes 3 arguments but 4 arguments were supplied" for `Encoder::start`.

- [ ] **Step 3: Give the encoder a frame rate.** Replace `src/capture/encode.rs` with:

```rust
//! The encoder thread: converts captured RGBA frames and writes them to a video file
//! with the linked FFmpeg libraries.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow, bail};
use ff::{Dictionary, Packet, Rational, codec, encoder, format, frame, software::scaling};
use ffmpeg_next as ff;

use crate::rate::FrameRate;

/// Frames that can wait for the encoder before real-time capture starts dropping them.
pub const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
    Hevc,
    /// FFV1 lossless RGB in Matroska, on the CPU.
    Ffv1,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 2] = [VideoFormat::Hevc, VideoFormat::Ffv1];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "HEVC NVENC 4:4:4, near-lossless",
            VideoFormat::Ffv1 => "FFV1 lossless, may drop frames above 720p",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "mp4",
            VideoFormat::Ffv1 => "mkv",
        }
    }
}

/// One captured frame: tightly packed sRGB RGBA rows, `width × height × 4` bytes.
pub struct Frame {
    /// Presentation time in frames at the recording's frame rate.
    pub pts: i64,
    pub rgba: Vec<u8>,
}

/// A running encoder thread. Dropping it without [`Encoder::finish`] still closes the file.
pub struct Encoder {
    frames: Option<SyncSender<Frame>>,
    recycled: Receiver<Vec<u8>>,
    thread: Option<JoinHandle<Result<u64>>>,
}

impl Encoder {
    /// Opens `path` for writing at `rate` frames per second and starts the encoder
    /// thread. Fails without leaving a file behind if the file exists or the encoder
    /// can't be opened.
    pub fn start(
        path: &Path,
        format: VideoFormat,
        size: (u32, u32),
        rate: FrameRate,
    ) -> Result<Self> {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let (frames, frames_rx) = mpsc::sync_channel(QUEUE);
        let (recycle_tx, recycled) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let owned = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("encoder".into())
            .spawn(move || run(owned, format, size, rate, frames_rx, recycle_tx, ready_tx))
            .context("could not start the encoder thread")?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                frames: Some(frames),
                recycled,
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                let _ = std::fs::remove_file(path);
                Err(err)
            }
            Err(_) => Err(anyhow!("the encoder thread stopped while opening")),
        }
    }

    /// Queues a frame without waiting. Returns the frame back if the queue is full.
    /// An error means the encoder has stopped; [`Encoder::finish`] reports why.
    pub fn try_send(&self, frame: Frame) -> Result<Option<Frame>> {
        match self.sender()?.try_send(frame) {
            Ok(()) => Ok(None),
            Err(TrySendError::Full(frame)) => Ok(Some(frame)),
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("the encoder stopped")),
        }
    }

    /// Queues a frame, waiting for room. An error means the encoder has stopped.
    pub fn send(&self, frame: Frame) -> Result<()> {
        self.sender()?
            .send(frame)
            .map_err(|_| anyhow!("the encoder stopped"))
    }

    /// A frame buffer the encoder has finished with, for reuse.
    pub fn recycled_buffer(&self) -> Option<Vec<u8>> {
        self.recycled.try_recv().ok()
    }

    /// Flushes the encoder, closes the file and returns the number of frames written,
    /// or the error that stopped the encoder.
    pub fn finish(mut self) -> Result<u64> {
        self.frames = None;
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow!("the encoder thread panicked")),
            None => Err(anyhow!("the encoder already finished")),
        }
    }

    fn sender(&self) -> Result<&SyncSender<Frame>> {
        self.frames
            .as_ref()
            .ok_or_else(|| anyhow!("the encoder already finished"))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.frames = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    path: PathBuf,
    format: VideoFormat,
    size: (u32, u32),
    rate: FrameRate,
    frames: Receiver<Frame>,
    recycled: Sender<Vec<u8>>,
    ready: SyncSender<Result<()>>,
) -> Result<u64> {
    let mut sink = match Sink::open(&path, format, size, rate) {
        Ok(sink) => {
            let _ = ready.send(Ok(()));
            sink
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            return Ok(0);
        }
    };
    let mut written = 0;
    for frame in frames {
        if let Err(err) = sink.write(&frame) {
            // Keep what was written playable where possible.
            let _ = sink.finish();
            return Err(err.context(format!("encoding frame {}", frame.pts)));
        }
        written += 1;
        let _ = recycled.send(frame.rgba);
    }
    sink.finish()?;
    Ok(written)
}

/// How captured RGBA becomes the encoder's pixel format.
enum Convert {
    /// RGBA → YUV 4:4:4 with BT.709 coefficients (HEVC).
    Scale {
        scaler: scaling::Context,
        rgba: frame::Video,
    },
    /// RGBA → BGRA by swapping bytes, which is exact (FFV1).
    Swizzle,
}

struct Sink {
    output: format::context::Output,
    encoder: encoder::Video,
    convert: Convert,
    frame: frame::Video,
    size: (u32, u32),
    /// One frame: the time base the frames' timestamps count in.
    time_base: Rational,
    stream_time_base: Rational,
}

impl Sink {
    fn open(path: &Path, format: VideoFormat, size: (u32, u32), rate: FrameRate) -> Result<Self> {
        ff::init().context("could not initialise FFmpeg")?;
        let (width, height) = size;
        let frame_rate = Rational(rate.num, rate.den);
        let time_base = frame_rate.invert();
        let (codec_name, pixel) = match format {
            VideoFormat::Hevc => ("hevc_nvenc", format::Pixel::YUV444P),
            VideoFormat::Ffv1 => ("ffv1", format::Pixel::BGRA),
        };
        let codec = encoder::find_by_name(codec_name)
            .ok_or_else(|| anyhow!("this FFmpeg build has no {codec_name} encoder"))?;
        let mut output =
            format::output(path).with_context(|| format!("could not create {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);
        let mut stream = output.add_stream(codec)?;
        let mut setup = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        setup.set_width(width);
        setup.set_height(height);
        setup.set_format(pixel);
        setup.set_time_base(time_base);
        setup.set_frame_rate(Some(frame_rate));
        setup.set_gop(rate.keyframe_interval());
        setup.set_max_b_frames(0);
        let mut options = Dictionary::new();
        match format {
            VideoFormat::Hevc => {
                setup.set_colorspace(ff::util::color::Space::BT709);
                setup.set_color_range(ff::util::color::Range::MPEG);
                setup.set_color_primaries(ff::util::color::Primaries::BT709);
                setup.set_color_transfer_characteristic(
                    ff::util::color::TransferCharacteristic::BT709,
                );
                for (key, value) in [
                    ("preset", "p4"),
                    ("tune", "hq"),
                    ("rc", "constqp"),
                    ("qp", "18"),
                    ("profile", "rext"),
                ] {
                    options.set(key, value);
                }
            }
            VideoFormat::Ffv1 => {
                // Without slice threads FFV1 runs on one core (about 8 fps at 1080p).
                setup.set_threading(codec::threading::Config {
                    kind: codec::threading::Type::Slice,
                    count: 0,
                });
                for (key, value) in [
                    ("level", "3"),
                    ("slices", "16"),
                    ("slicecrc", "1"),
                    ("coder", "1"),
                ] {
                    options.set(key, value);
                }
            }
        }
        if global_header {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = setup.open_with(options).map_err(|err| match format {
            VideoFormat::Hevc => anyhow!(
                "could not open the NVENC HEVC encoder ({err}); it needs an NVIDIA GPU with \
                 NVENC and a free encode session. Try FFV1 instead."
            ),
            VideoFormat::Ffv1 => anyhow!("could not open the FFV1 encoder ({err})"),
        })?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // Declare the constant frame rate so players don't have to infer it.
        stream.set_rate(frame_rate);
        stream.set_avg_frame_rate(frame_rate);
        let mut header = Dictionary::new();
        if format == VideoFormat::Hevc {
            // Fragmented MP4 stays playable if the recording is interrupted.
            header.set("movflags", "frag_keyframe+empty_moov");
        }
        output
            .write_header_with(header)
            .context("could not write the file header")?;
        let stream_time_base = output.stream(0).context("no video stream")?.time_base();

        let convert = match format {
            VideoFormat::Hevc => {
                let mut scaler = scaling::Context::get(
                    format::Pixel::RGBA,
                    width,
                    height,
                    pixel,
                    width,
                    height,
                    scaling::Flags::BILINEAR,
                )?;
                // swscale defaults to BT.601; match the BT.709 tags (limited range out).
                // SAFETY: the scaler pointer is valid for the life of `scaler`, and the
                // coefficient table returned by FFmpeg is static.
                unsafe {
                    let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
                    ff::ffi::sws_setColorspaceDetails(
                        scaler.as_mut_ptr(),
                        bt709,
                        1,
                        bt709,
                        0,
                        0,
                        1 << 16,
                        1 << 16,
                    );
                }
                Convert::Scale {
                    scaler,
                    rgba: frame::Video::new(format::Pixel::RGBA, width, height),
                }
            }
            VideoFormat::Ffv1 => Convert::Swizzle,
        };
        Ok(Self {
            output,
            encoder,
            convert,
            frame: frame::Video::new(pixel, width, height),
            size,
            time_base,
            stream_time_base,
        })
    }

    fn write(&mut self, captured: &Frame) -> Result<()> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let row = width * 4;
        if captured.rgba.len() != row * height {
            bail!("captured frame has the wrong size");
        }
        // The encoder may still hold a reference to the last frame's buffer.
        // SAFETY: `self.frame` owns a valid AVFrame.
        if unsafe { ff::ffi::av_frame_make_writable(self.frame.as_mut_ptr()) } < 0 {
            bail!("out of memory for the encoder frame");
        }
        match &mut self.convert {
            Convert::Swizzle => {
                let stride = self.frame.stride(0);
                let data = self.frame.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    let dst = &mut data[y * stride..y * stride + row];
                    let (dst, _) = dst.as_chunks_mut::<4>();
                    let (src, _) = src.as_chunks::<4>();
                    for (d, [r, g, b, a]) in dst.iter_mut().zip(src) {
                        *d = [*b, *g, *r, *a];
                    }
                }
            }
            Convert::Scale { scaler, rgba } => {
                let stride = rgba.stride(0);
                let data = rgba.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    data[y * stride..y * stride + row].copy_from_slice(src);
                }
                scaler.run(rgba, &mut self.frame)?;
            }
        }
        self.frame.set_pts(Some(captured.pts));
        self.encoder.send_frame(&self.frame)?;
        self.drain()
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        let mut packet = Packet::empty();
        loop {
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.set_duration(1); // one frame; the MP4 muxer warns without it
                    packet.rescale_ts(self.time_base, self.stream_time_base);
                    packet.write_interleaved(&mut self.output)?;
                }
                // EAGAIN means the encoder wants more input; Eof follows the final flush.
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.encoder.send_eof()?;
        self.drain()?;
        self.output.write_trailer()?;
        Ok(())
    }
}
```

The crate won't compile again until Step 7: the recorder and the app still use the old signatures.

- [ ] **Step 4: Number captured frames at the rate.** Replace `src/capture/mod.rs` with the following. The real-time decimation and its tests go away; `FrameClock` keeps a stop-after test, now also at 23.976 fps.

```rust
//! Video capture: recording the clean canvas output to a file.

pub mod encode;
pub mod readback;
pub mod recorder;

use encode::VideoFormat;

use crate::rate::FrameRate;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureMode {
    /// Record every canvas frame as it's drawn in real time; frames the encoder can't keep
    /// up with are dropped and counted.
    #[default]
    RealTime,
    /// Draw one canvas frame per screen refresh, whatever the time, and record every
    /// frame, waiting for the encoder when it falls behind.
    Offline,
}

impl CaptureMode {
    pub const ALL: [CaptureMode; 2] = [CaptureMode::RealTime, CaptureMode::Offline];

    pub fn label(self) -> &'static str {
        match self {
            CaptureMode::RealTime => "Real-time",
            CaptureMode::Offline => "Offline (frame-accurate)",
        }
    }
}

/// `rasterwarp-YYYYMMDD-HHMMSS.<ext>` for a recording started at `now` (local time).
pub fn file_name(now: chrono::NaiveDateTime, format: VideoFormat) -> String {
    format!(
        "rasterwarp-{}.{}",
        now.format("%Y%m%d-%H%M%S"),
        format.extension()
    )
}

/// `folder/name`, or `folder/stem-2.ext`, `stem-3.ext`, … if that file already exists.
pub fn unused_path(folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let first = folder.join(name);
    if !first.exists() {
        return first;
    }
    let path = std::path::Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(String::new, |e| format!(".{e}"));
    (2..)
        .map(|n| folder.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("an unbounded range always finds a free name")
}

/// Hands out a recording's timestamps: each captured canvas frame is the next frame of
/// the file.
#[derive(Clone, Debug)]
pub struct FrameClock {
    rate: FrameRate,
    /// The next timestamp to hand out; also the recording's length in frames.
    next: i64,
}

impl FrameClock {
    pub fn new(rate: FrameRate) -> Self {
        Self { rate, next: 0 }
    }

    /// The timestamp for the next captured frame.
    pub fn next_pts(&mut self) -> i64 {
        let pts = self.next;
        self.next += 1;
        pts
    }

    /// The recording's length in frames so far.
    pub fn frames(&self) -> i64 {
        self.next
    }

    /// The recording's length in seconds so far.
    pub fn seconds(&self) -> f64 {
        self.rate.seconds(self.next)
    }

    /// Whether a recording limited to `seconds` (0 = no limit) is long enough. Counted
    /// in whole frames, so 10/60 s at 60 fps stops after exactly 10 frames.
    pub fn reached(&self, seconds: f32) -> bool {
        seconds > 0.0 && self.next >= self.rate.frames_in(f64::from(seconds))
    }
}

/// Bytes per row in a texture-to-buffer copy: `width × 4` rounded up to 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Copies `height` rows of `width` RGBA pixels out of a padded buffer into `out`.
pub fn unpad(padded: &[u8], width: u32, height: u32, out: &mut Vec<u8>) {
    let row = width as usize * 4;
    let stride = padded_bytes_per_row(width) as usize;
    out.clear();
    for r in padded.chunks(stride).take(height as usize) {
        out.extend_from_slice(&r[..row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_after_the_start_time() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_opt(9, 5, 3)
            .unwrap();
        assert_eq!(
            file_name(t, VideoFormat::Hevc),
            "rasterwarp-20261007-090503.mp4"
        );
        assert_eq!(
            file_name(t, VideoFormat::Ffv1),
            "rasterwarp-20261007-090503.mkv"
        );
    }

    #[test]
    fn a_taken_file_name_gets_a_numeric_suffix() {
        let folder = std::env::temp_dir().join(format!("rasterwarp-unused-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        assert_eq!(unused_path(&folder, "a.mp4"), folder.join("a.mp4"));
        std::fs::write(folder.join("a.mp4"), b"").unwrap();
        assert_eq!(unused_path(&folder, "a.mp4"), folder.join("a-2.mp4"));
        std::fs::write(folder.join("a-2.mp4"), b"").unwrap();
        assert_eq!(unused_path(&folder, "a.mp4"), folder.join("a-3.mp4"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn timestamps_count_captured_frames() {
        let mut clock = FrameClock::new(FrameRate::whole(60));
        let pts: Vec<i64> = (0..5).map(|_| clock.next_pts()).collect();
        assert_eq!(pts, [0, 1, 2, 3, 4]);
        assert_eq!(clock.frames(), 5);
    }

    #[test]
    fn seconds_follow_the_frame_rate() {
        let mut clock = FrameClock::new(FrameRate::ntsc(24));
        for _ in 0..24 {
            clock.next_pts();
        }
        assert!((clock.seconds() - 1.001).abs() < 1e-9);
    }

    #[test]
    fn stop_after_counts_whole_frames() {
        let mut clock = FrameClock::new(FrameRate::whole(60));
        for _ in 0..9 {
            clock.next_pts();
        }
        assert!(!clock.reached(10.0 / 60.0));
        clock.next_pts();
        assert!(clock.reached(10.0 / 60.0));
        assert!(!clock.reached(0.0), "0 means no limit");
        let mut ntsc = FrameClock::new(FrameRate::ntsc(24));
        for _ in 0..23 {
            ntsc.next_pts();
        }
        assert!(!ntsc.reached(1.0));
        ntsc.next_pts();
        assert!(ntsc.reached(1.0), "1 s at 23.976 fps is 24 frames");
    }

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn unpad_drops_the_row_padding() {
        let (w, h) = (65, 2);
        let stride = padded_bytes_per_row(w) as usize;
        let mut padded = vec![0u8; stride * h as usize];
        padded[0] = 1;
        padded[w as usize * 4 - 1] = 2;
        padded[stride] = 3;
        let mut out = Vec::new();
        unpad(&padded, w, h, &mut out);
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert_eq!(
            (out[0], out[w as usize * 4 - 1], out[w as usize * 4]),
            (1, 2, 3)
        );
    }
}
```

- [ ] **Step 5: Record every captured frame.** Replace `src/capture/recorder.rs` with:

```rust
//! One recording: reads captured canvas frames back from the GPU and feeds them to the
//! encoder thread.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use super::encode::{Encoder, Frame, VideoFormat};
use super::readback::{Readback, staging_bytes};
use super::{CaptureMode, FrameClock, file_name, unused_path};
use crate::rate::FrameRate;

/// What the panel chose before pressing Record.
#[derive(Clone, Debug)]
pub struct RecordSettings {
    pub format: VideoFormat,
    pub mode: CaptureMode,
    pub folder: PathBuf,
    /// Stop automatically after this many seconds of video (0 = stop by hand).
    pub stop_after: f32,
}

/// Progress shown while recording.
#[derive(Clone, Debug)]
pub struct RecordStatus {
    pub path: PathBuf,
    /// Seconds of video recorded so far.
    pub seconds: f64,
    /// Frames handed to the encoder.
    pub frames: u64,
    /// Frames lost because the encoder fell behind (real-time only).
    pub dropped: u64,
    /// Offline only: seconds of video per second of wall time.
    pub speed: Option<f64>,
}

pub struct Recorder {
    encoder: Encoder,
    readback: Readback,
    clock: FrameClock,
    mode: CaptureMode,
    stop_after: f32,
    path: PathBuf,
    started: Instant,
    frames: u64,
    dropped: u64,
    /// Frame buffers ready for reuse.
    spare: Vec<Vec<u8>>,
}

impl Recorder {
    /// Opens a new file in the settings' folder and starts encoding at `rate` frames per
    /// second.
    pub fn start(
        device: &wgpu::Device,
        settings: &RecordSettings,
        canvas: (u32, u32),
        rate: FrameRate,
    ) -> Result<Self> {
        if staging_bytes(canvas) > device.limits().max_buffer_size {
            bail!(
                "{}×{} is too large to record on this GPU; choose a smaller canvas",
                canvas.0,
                canvas.1
            );
        }
        std::fs::create_dir_all(&settings.folder)
            .with_context(|| format!("could not create {}", settings.folder.display()))?;
        let now = chrono::Local::now().naive_local();
        let path = unused_path(&settings.folder, &file_name(now, settings.format));
        let encoder = Encoder::start(&path, settings.format, canvas, rate)?;
        Ok(Self {
            encoder,
            readback: Readback::new(device, canvas),
            clock: FrameClock::new(rate),
            mode: settings.mode,
            stop_after: settings.stop_after,
            path,
            started: Instant::now(),
            frames: 0,
            dropped: 0,
            spare: Vec::new(),
        })
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Call once per frame before rendering: hands frames that have come back from the
    /// GPU to the encoder. An error means the encoder has stopped.
    pub fn pump(&mut self, device: &wgpu::Device) -> Result<()> {
        self.deliver(device, false, self.mode == CaptureMode::Offline)
    }

    /// Captures the canvas frame just drawn as the next frame of the file: `draw` must
    /// draw the capture composite into the given view. When every staging buffer is
    /// still busy this waits for the GPU. Submit the frame's commands and call
    /// [`Recorder::after_submit`] before capturing the next frame.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<()> {
        let pts = self.clock.next_pts();
        draw(encoder, self.readback.view());
        if self.readback.copy(encoder, pts) {
            return Ok(());
        }
        // Waiting for the GPU is brief; only the encoder's queue may drop frames.
        self.deliver(device, true, self.mode == CaptureMode::Offline)?;
        if !self.readback.copy(encoder, pts) {
            bail!("no capture buffer came free");
        }
        Ok(())
    }

    /// Call after submitting the frame's commands.
    pub fn after_submit(&mut self) {
        self.readback.after_submit();
    }

    /// Whether the "stop after" length has been reached.
    pub fn finished_recording(&self) -> bool {
        self.clock.reached(self.stop_after)
    }

    pub fn status(&self) -> RecordStatus {
        let seconds = self.clock.seconds();
        RecordStatus {
            path: self.path.clone(),
            seconds,
            frames: self.frames,
            dropped: self.dropped,
            speed: (self.mode == CaptureMode::Offline)
                .then(|| seconds / self.started.elapsed().as_secs_f64().max(1e-3)),
        }
    }

    /// Waits for the frames still on the GPU, encodes them, closes the file, and returns
    /// the final status or the error that stopped the encoder.
    pub fn finish(mut self, device: &wgpu::Device) -> Result<RecordStatus> {
        while self.readback.is_busy() {
            if let Err(err) = self.deliver(device, true, true) {
                return Err(self.encoder.finish().err().unwrap_or(err));
            }
        }
        let status = self.status();
        let written = self.encoder.finish()?;
        log::info!("recorded {written} frames to {}", status.path.display());
        Ok(status)
    }

    /// Sends finished readbacks to the encoder. Waits for the GPU when `wait` is set, and
    /// for room in the encoder's queue when `block` is set (otherwise a full queue drops
    /// the frame).
    fn deliver(&mut self, device: &wgpu::Device, wait: bool, block: bool) -> Result<()> {
        while let Some(buffer) = self.encoder.recycled_buffer() {
            self.spare.push(buffer);
        }
        let spare = &mut self.spare;
        let frames = self
            .readback
            .collect(device, wait, || spare.pop().unwrap_or_default())?;
        for frame in frames {
            self.send(frame, block)?;
        }
        Ok(())
    }

    fn send(&mut self, frame: Frame, block: bool) -> Result<()> {
        if block {
            self.encoder.send(frame)?;
        } else if let Some(frame) = self.encoder.try_send(frame)? {
            self.dropped += 1;
            self.spare.push(frame.rgba);
            return Ok(());
        }
        self.frames += 1;
        Ok(())
    }
}
```

- [ ] **Step 6: Update the smoke tests.** Replace `tests/smoke.rs` with the following. `records_every_frame_offline` now records 0.5 s at 23.976 fps (12 frames). `real_time_capture_counts_frames_it_has_to_drop` becomes `real_time_capture_waits_for_the_gpu_when_the_ring_is_full`.

```rust
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
```

- [ ] **Step 7: Keep the app compiling.** Replace `src/app.rs` with the following. It records at `FrameRate::default()`, uses that rate's period for offline steps, and doesn't record paused frames.

```rust
//! The windowed application: owns the GPU context, renderer, parameters and UI,
//! and runs one frame per redraw.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::canvas::{self, CanvasChoice};
use crate::capture::CaptureMode;
use crate::capture::recorder::Recorder;
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::rate::FrameRate;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(state) => self.state = Some(state),
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let response = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                state.stop_recording(); // finish the file before exiting
                event_loop.exit();
            }
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
            // Space triggers a transition unless egui is using the keyboard (e.g. a text field).
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && event.logical_key == Key::Named(NamedKey::Space)
                    && !response.consumed
                    && !state.egui_ctx.egui_wants_keyboard_input() =>
            {
                state.motion.trigger();
            }
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_side: u32,
    composite_format: wgpu::TextureFormat,
    renderer: Renderer,
    preview: PreviewView,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    motion: Motion,
    recorder: Option<Recorder>,
    ui: UiState,
    time: f64,
    last_frame: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .with_context(|| {
                format!(
                    "failed to create window surface for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl"
                )
            })?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;
        let max_texture_side = device.limits().max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects). Where the adapter allows an
        // sRGB view of it, the composite pass draws through that view; otherwise (e.g. GL)
        // it draws into the plain format and encodes sRGB in the shader.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        let srgb_view_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let composite_format = if srgb_view_ok { srgb_format } else { format };
        // Fifo (vsync) keeps the per-frame feedback semantics at 60 fps. RASTERWARP_UNCAPPED=1
        // opts into Mailbox (uncapped, no tearing) to measure the real render cost.
        let uncapped = std::env::var("RASTERWARP_UNCAPPED").is_ok_and(|v| v == "1");
        let present_mode = if uncapped && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = if srgb_view_ok {
            vec![srgb_format]
        } else {
            vec![]
        };
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {composite_format:?}), {present_mode:?}");

        let mut ui = UiState {
            frame_ms: 16.7,
            show_preview: true,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match load_fitting(path, max_texture_side) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
        let renderer = Renderer::new(&device, &queue, composite_format, canvas::DEFAULT, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview =
            PreviewView::new(&device, &queue, &mut egui_renderer, canvas::DEFAULT, &image);

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
            egui_ctx,
            egui_state,
            egui_renderer,
            motion: Motion::new(Params::default()),
            recorder: None,
            ui,
            time: 0.0,
            last_frame: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.preview.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    /// Advances animation time: by the frame time, or by exactly one frame period per
    /// frame while recording offline.
    fn advance_animation(&mut self, dt: f32) {
        if self.ui.paused {
            return;
        }
        let offline = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.mode() == CaptureMode::Offline);
        // Clamp so a stall (e.g. dragging the window) doesn't make animation jump.
        let step = if offline {
            FrameRate::default().period() as f32
        } else {
            dt.min(0.1)
        };
        self.time += f64::from(step);
        self.motion.advance(step);
    }

    fn start_recording(&mut self) {
        self.ui.capture.saved = None;
        match Recorder::start(
            &self.device,
            &self.ui.capture.settings,
            self.renderer.size(),
            FrameRate::default(),
        ) {
            Ok(recorder) => {
                // Opening the encoder stalls; that must not count as animation time.
                self.last_frame = Instant::now();
                self.ui.capture.error = None;
                self.ui.capture.status = Some(recorder.status());
                self.recorder = Some(recorder);
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("{err:#}"));
            }
        }
    }

    /// Finishes the current recording, if any, and reports how it went.
    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        self.ui.capture.status = None;
        let result = recorder.finish(&self.device);
        // Flushing the encoder stalls; that must not count as animation time.
        self.last_frame = Instant::now();
        match result {
            Ok(status) => {
                self.ui.capture.saved = Some(format!(
                    "Saved {} ({:.1} s, {} frames, {} dropped)",
                    status.path.display(),
                    status.seconds,
                    status.frames,
                    status.dropped
                ));
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("Recording stopped: {err:#}"));
            }
        }
    }

    fn choose_capture_folder(&mut self) {
        let folder = &mut self.ui.capture.settings.folder;
        let mut dialog = rfd::FileDialog::new().set_title("Capture folder");
        if let Ok(start) = std::path::absolute(&*folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            *folder = picked;
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;

        if self.config.width == 0 || self.config.height == 0 {
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        // Animation moves only for frames that are drawn, so offline recordings stay exact.
        self.advance_animation(dt);
        if let Some(recorder) = &mut self.recorder
            && let Err(err) = recorder.pump(&self.device)
        {
            log::warn!("{err:#}");
            self.stop_recording();
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        if actions.choose_folder {
            self.choose_capture_folder();
        }
        if actions.stop_recording {
            self.stop_recording();
        }
        if let Some(size) = actions.apply_canvas
            && self.recorder.is_none()
        {
            self.renderer.resize(&self.device, size);
            self.preview
                .resize(&self.device, &mut self.egui_renderer, size);
            self.ui.canvas.current = size;
        }
        if actions.start_recording && self.recorder.is_none() {
            self.start_recording();
        }
        let frame_params = self.motion.frame();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
        }
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &frame_params,
            self.time as f32,
            &srgb_view,
            output_size,
        );
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
        let mut capture_failed = false;
        if let Some(recorder) = &mut self.recorder
            && !self.ui.paused
        {
            let (device, queue, renderer, time) =
                (&self.device, &self.queue, &self.renderer, self.time);
            let captured = recorder.capture(device, &mut encoder, |encoder, view| {
                renderer.composite_capture(device, queue, encoder, &frame_params, time as f32, view)
            });
            if let Err(err) = captured {
                log::warn!("{err:#}");
                capture_failed = true;
            }
        }
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        if let Some(recorder) = &mut self.recorder {
            recorder.after_submit();
            self.ui.capture.status = Some(recorder.status());
            if capture_failed || recorder.finished_recording() {
                self.stop_recording();
            }
        }
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}

/// Loads an image and checks it fits in a GPU texture.
fn load_fitting(path: &Path, max_side: u32) -> anyhow::Result<GrayImage> {
    let image = source::load_image(path)?;
    source::ensure_fits(&image, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}
```

- [ ] **Step 8: Run everything.** Run: `cargo test`. Expected: `119 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `6 passed` in `tests/smoke.rs`. On a machine without NVENC, the HEVC test prints that it is skipping. `cargo test --lib capture::` alone: `11 passed`.

- [ ] **Step 9: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add src/capture/encode.rs src/capture/mod.rs src/capture/recorder.rs src/app.rs tests/capture.rs tests/smoke.rs
git commit -m "feat: record at the program frame rate" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Split drawing a canvas frame from showing it

**Files:**
- Modify: `src/passes/mod.rs`, `tests/smoke.rs`
- Test: `compositing_again_shows_the_same_canvas_frame` in `tests/smoke.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Renderer::render_canvas(device, queue, encoder, &FrameParams, time)`: warp, colorize, feedback and bloom. It advances the trails by one frame.
  - `Renderer::composite(&self, device, queue, encoder, &FrameParams, time, output, output_size)`: shows the current canvas frame without touching it.
  - `Renderer::render` is now `render_canvas` followed by `composite`. The preview and the existing tests keep using it.
  - `Renderer::composite_capture` is unchanged; it reads the current canvas frame too.

- [ ] **Step 1: Write the failing test.** Replace `tests/smoke.rs` with the following (adds `compositing_again_shows_the_same_canvas_frame` before `renders_the_preview_offscreen`):

```rust
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
```

- [ ] **Step 2: Run it and confirm it fails.** Run: `cargo test --test smoke`. Expected: compile errors such as "no method named `render_canvas` found for struct `Renderer`".

- [ ] **Step 3: Implement.** Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::blend::FrameParams;
use crate::source::{self, GrayImage};

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
    /// Draws the same composite into a canvas-sized capture texture.
    capture: composite::CompositePass,
}

impl Renderer {
    /// `size` is the internal (canvas) resolution; `output_format` is the format of the
    /// texture `render` draws into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        size: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let (w, h) = size;
        Self {
            size,
            source: source::upload(device, queue, image).create_view(&Default::default()),
            source_aspect: image.aspect(),
            warp: warp::WarpPass::new(device, w, h),
            colorize: colorize::ColorizePass::new(device, w, h),
            feedback: feedback::FeedbackPass::new(device, w, h),
            bloom: bloom::BloomPass::new(device, w, h),
            composite: composite::CompositePass::new(device, output_format),
            capture: composite::CompositePass::new(device, CAPTURE_FORMAT),
        }
    }

    /// The internal (canvas) resolution.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Rebuilds every canvas-sized target at `size`. The trails start out cleared.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (w, h) = size;
        self.size = size;
        self.warp = warp::WarpPass::new(device, w, h);
        self.colorize = colorize::ColorizePass::new(device, w, h);
        self.feedback = feedback::FeedbackPass::new(device, w, h);
        self.bloom = bloom::BloomPass::new(device, w, h);
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source = source::upload(device, queue, image).create_view(&Default::default());
        self.source_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`: draws a canvas frame, then composites it into
    /// `output` (`output_size` pixels, letterboxed).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        self.render_canvas(device, queue, encoder, frame, time);
        self.composite(device, queue, encoder, frame, time, output, output_size);
    }

    /// Draws the next canvas frame (warp, colorize, feedback, bloom) without showing it.
    /// Each call advances the feedback trails by one frame.
    pub fn render_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect),
        );
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&frame.colorize),
        );
        self.feedback.render(
            device,
            queue,
            encoder,
            &self.colorize.target.view,
            &feedback::uniforms(&frame.feedback, aspect),
        );
        self.bloom.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            frame.glow.bloom_threshold,
        );
    }

    /// Composites the current canvas frame into `output` (`output_size` pixels,
    /// letterboxed). Doesn't touch the canvas, so it can run any number of times per
    /// canvas frame.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                output_size,
                self.composite.encode_srgb(),
            ),
        );
    }

    /// Draws the current canvas frame's composite into `output`: a canvas-sized texture
    /// in [`CAPTURE_FORMAT`], with no letterboxing and no UI.
    pub fn composite_capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
    ) {
        self.capture.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                self.size,
                self.capture.encode_srgb(),
            ),
        );
    }
}
```

- [ ] **Step 4: Run the tests and confirm they pass.** Run: `cargo test --test smoke`. Expected: `7 passed`. Then `cargo test`: `119 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `7 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/passes/mod.rs tests/smoke.rs
git commit -m "refactor: split drawing a canvas frame from compositing it" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Pace the app by the frame rate

**Files:**
- Modify: `src/app.rs`, `src/ui.rs`
- Test: full suite; manual checklist with real recordings

**Interfaces:**
- Consumes: `FrameRate`, `CanvasClock`, `Pacing`, `MAX_CATCH_UP` (Task 1); `Recorder::start(.., rate)` and `Recorder::capture(device, encoder, draw)` (Task 2); `Renderer::render_canvas` and `Renderer::composite` (Task 3).
- Produces:
  - `UiState.rate: FrameRate` (the panel's choice) and `UiState.late: u64` (shown in the header).
  - `UiState.frame_ms` now means the time between screen refreshes.
  - `ui::draw` shows a **Frame rate** menu next to Canvas. It is disabled while recording.
  - Each refresh in the app:
    1. runs the UI and its actions, applying a rate change while not recording;
    2. asks `CanvasClock::due` how many canvas frames to draw (`Pacing::Offline` while recording offline);
    3. draws each one with `draw_canvas_frame`, which advances animation, draws the canvas and the preview, captures, and makes its own submit;
    4. composites the newest canvas frame with the UI on top.
  - Recorder start and stop, the folder dialog and startup call `CanvasClock::reanchor`.

- [ ] **Step 1: Add the Frame rate menu and readouts.** Replace `src/ui.rs` with:

```rust
//! The egui parameter panel.

use std::path::PathBuf;

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::canvas::{CanvasChoice, PRESETS};
use crate::capture::CaptureMode;
use crate::capture::encode::VideoFormat;
use crate::capture::recorder::{RecordSettings, RecordStatus};
use crate::curve::{CurveLibrary, CurveRef};
use crate::curve_editor::curve_editor;
use crate::motion::{Mode, Motion};
use crate::params::{Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, ranges};
use crate::rate::FrameRate;
use crate::sequence::{FRAMES_PER_SECOND, MAX_FRAME};
use crate::transition::{AbState, DURATION};

/// UI-only state that isn't a render parameter.
#[derive(Default)]
pub struct UiState {
    pub paused: bool,
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// Smoothed time between screen refreshes in milliseconds.
    pub frame_ms: f32,
    /// The program frame rate chosen in the panel.
    pub rate: FrameRate,
    /// Canvas frames skipped so far because the app fell behind.
    pub late: u64,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
    pub canvas: CanvasChoice,
    pub capture: CaptureUi,
}

/// The capture controls and what the current or last recording reported.
#[derive(Clone, Debug)]
pub struct CaptureUi {
    pub settings: RecordSettings,
    /// Progress while recording.
    pub status: Option<RecordStatus>,
    /// What the last recording saved.
    pub saved: Option<String>,
    pub error: Option<String>,
}

impl Default for CaptureUi {
    fn default() -> Self {
        Self {
            settings: RecordSettings {
                format: VideoFormat::Hevc,
                mode: CaptureMode::RealTime,
                folder: PathBuf::from("captures"),
                stop_after: 0.0,
            },
            status: None,
            saved: None,
            error: None,
        }
    }
}

/// The off-air preview as the panel shows it.
pub struct PreviewOverlay {
    pub texture: egui::TextureId,
    /// Texture size in pixels.
    pub size: (u32, u32),
    pub label: String,
}

/// Display width of the preview overlay, in points.
const PREVIEW_WIDTH: f32 = 320.0;

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
    /// Switch the canvas to this size.
    pub apply_canvas: Option<(u32, u32)>,
    pub start_recording: bool,
    pub stop_recording: bool,
    pub choose_folder: bool,
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
    let mut actions = UiActions::default();
    egui::Panel::left("controls")
        .resizable(true)
        .default_size(320.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Rasterwarp");
                ui.label(format!(
                    "{} · display {:.0} Hz · {} late",
                    state.rate.label(),
                    1000.0 / state.frame_ms.max(0.001),
                    state.late
                ));
                ui.label(&state.source_info);
                if let Some(err) = &state.load_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                ui.label("Drop a PNG/JPG onto the window to load it.");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.paused, "Pause");
                    ui.checkbox(&mut state.show_preview, "Show preview");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                let recording = state.capture.status.is_some();
                capture_section(ui, &mut state.capture, &mut actions);
                rate_section(ui, &mut state.rate, recording);
                canvas_section(ui, &mut state.canvas, recording, &mut actions);
                mode_section(ui, motion);
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
}

/// The preview, framed and labelled, in the bottom-right corner of the window.
fn preview_overlay(ctx: &egui::Context, preview: &PreviewOverlay) {
    let height = PREVIEW_WIDTH * preview.size.1 as f32 / preview.size.0 as f32;
    egui::Area::new(egui::Id::new("preview"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(&preview.label);
                ui.image((preview.texture, egui::vec2(PREVIEW_WIDTH, height)));
            });
        });
}

fn capture_section(ui: &mut Ui, capture: &mut CaptureUi, actions: &mut UiActions) {
    CollapsingHeader::new("Capture")
        .default_open(true)
        .show(ui, |ui| {
            let settings = &mut capture.settings;
            ui.add_enabled_ui(capture.status.is_none(), |ui| {
                ComboBox::from_id_salt("capture format")
                    .selected_text(settings.format.label())
                    .show_ui(ui, |ui| {
                        for format in VideoFormat::ALL {
                            ui.selectable_value(&mut settings.format, format, format.label());
                        }
                    });
                ComboBox::from_id_salt("capture mode")
                    .selected_text(settings.mode.label())
                    .show_ui(ui, |ui| {
                        for mode in CaptureMode::ALL {
                            ui.selectable_value(&mut settings.mode, mode, mode.label());
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Stop after");
                    ui.add(
                        egui::DragValue::new(&mut settings.stop_after)
                            .range(0.0..=3600.0)
                            .speed(0.1)
                            .suffix(" s"),
                    );
                    ui.small("(0 = stop by hand)");
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Folder: {}", settings.folder.display()));
                    if ui.button("Choose…").clicked() {
                        actions.choose_folder = true;
                    }
                });
            });
            match &capture.status {
                None => {
                    if ui.button("Record").clicked() {
                        actions.start_recording = true;
                    }
                }
                Some(status) => {
                    if ui.button("Stop recording").clicked() {
                        actions.stop_recording = true;
                    }
                    ui.label(format!(
                        "{:.1} s · {} frames · {} dropped",
                        status.seconds, status.frames, status.dropped
                    ));
                    if let Some(speed) = status.speed {
                        ui.label(format!("offline: {speed:.2}× realtime"));
                    } else if status.dropped > 0 {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "The encoder can't keep up, so frames are being dropped. Offline mode records every frame.",
                        );
                    }
                }
            }
            if let Some(saved) = &capture.saved {
                ui.small(saved);
            }
            if let Some(err) = &capture.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
            }
        });
}

/// The program frame rate: canvas frames and recordings both run at it.
fn rate_section(ui: &mut Ui, rate: &mut FrameRate, recording: bool) {
    ui.add_enabled_ui(!recording, |ui| {
        ui.horizontal(|ui| {
            ui.label("Frame rate");
            ComboBox::from_id_salt("frame rate")
                .selected_text(rate.label())
                .show_ui(ui, |ui| {
                    for choice in FrameRate::ALL {
                        ui.selectable_value(rate, choice, choice.label());
                    }
                });
        });
    });
}

fn canvas_section(
    ui: &mut Ui,
    canvas: &mut CanvasChoice,
    recording: bool,
    actions: &mut UiActions,
) {
    CollapsingHeader::new("Canvas").show(ui, |ui| {
        let name = |i: usize| match PRESETS.get(i) {
            Some((name, (w, h))) => format!("{name} ({w}×{h})"),
            None => "Custom".to_owned(),
        };
        ComboBox::from_id_salt("canvas preset")
            .selected_text(name(canvas.choice))
            .show_ui(ui, |ui| {
                for i in 0..=CanvasChoice::CUSTOM {
                    ui.selectable_value(&mut canvas.choice, i, name(i));
                }
            });
        if canvas.choice == CanvasChoice::CUSTOM {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut canvas.custom.0).range(1..=canvas.max_side));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut canvas.custom.1).range(1..=canvas.max_side));
            });
        }
        let (w, h) = canvas.requested();
        let (cw, ch) = canvas.current;
        ui.label(format!("Rendering at {cw}×{ch}"));
        ui.horizontal(|ui| {
            let changed = (w, h) != canvas.current;
            if ui
                .add_enabled(changed && !recording, Button::new(format!("Apply {w}×{h}")))
                .clicked()
            {
                actions.apply_canvas = Some((w, h));
            }
            ui.small(if recording {
                "stop recording to change"
            } else {
                "clears the trails"
            });
        });
    });
}

fn mode_section(ui: &mut Ui, motion: &mut Motion) {
    ui.separator();
    ui.horizontal(|ui| {
        for mode in Mode::ALL {
            if ui
                .selectable_label(motion.mode() == mode, format!("{mode:?}"))
                .clicked()
            {
                motion.set_mode(mode);
            }
        }
    });
    match motion.mode() {
        Mode::Live => {
            ui.label("Editing what's on screen.");
        }
        Mode::Transition => transition_controls(ui, motion),
        Mode::Sequence => sequence_controls(ui, motion),
    }
    ui.separator();
}

fn transition_controls(ui: &mut Ui, motion: &mut Motion) {
    let ab = &motion.ab;
    ui.label(format!(
        "On air: {} · editing: {}",
        AbState::bank_name(ab.on_air),
        AbState::bank_name(ab.off_air())
    ));
    let label = match ab.ramp() {
        Some(r) if r.forward => "Reverse (Space)",
        Some(_) => "Resume (Space)",
        None => "Transition (Space)",
    };
    ui.horizontal(|ui| {
        if ui.button(label).clicked() {
            motion.trigger();
        }
        if ui.button("Cut").clicked() {
            motion.cut();
        }
    });
    let progress = motion.ab.ramp().map_or(0.0, |r| r.progress);
    ui.add(ProgressBar::new(progress).show_percentage());
    ui.add(
        Slider::new(&mut motion.ab.duration, DURATION)
            .text("duration (s)")
            .logarithmic(true),
    );
    curve_picker(ui, "ab curve", &motion.curves, &mut motion.ab.curve);
}

fn sequence_controls(ui: &mut Ui, motion: &mut Motion) {
    let curves = motion.curves.clone();
    let seq = motion.sequence_mut();
    ui.horizontal(|ui| {
        if seq.is_running() {
            if ui.button("Stop").clicked() {
                seq.stop();
            }
        } else if ui.button("Run").clicked() {
            seq.run();
        }
        if ui.button("Reset").clicked() {
            seq.reset();
        }
        ui.checkbox(&mut seq.looping, "Loop");
    });
    let frame = seq.clock_frames();
    ui.label(format!(
        "Frame {:.0} ({:.2} s at {FRAMES_PER_SECOND} fps)",
        frame,
        frame / FRAMES_PER_SECOND
    ));
    for i in 0..seq.cues().len() {
        let text = format!("Cue {} @ frame {}", i + 1, seq.cues()[i].start_frame);
        if ui.selectable_label(seq.selected == i, text).clicked() {
            seq.selected = i;
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Add cue").clicked() {
            seq.add_cue();
        }
        let can_delete = seq.cues().len() > 1;
        if ui
            .add_enabled(can_delete, Button::new("Delete cue"))
            .clicked()
        {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    // Cue 1 always starts at frame 0 and is never ramped into, so its controls stay disabled.
    ui.add_enabled_ui(i > 0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
                .changed()
            {
                seq.set_start_frame(i, start);
            }
            ui.label(format!("{:.2} s", start as f32 / FRAMES_PER_SECOND));
        });
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
                .changed()
            {
                seq.set_duration(i, duration);
            }
            ui.label(format!("{:.2} s", duration as f32 / FRAMES_PER_SECOND));
        });
        curve_picker(ui, "cue curve", &curves, &mut seq.selected_cue_mut().curve);
    });
    if seq.is_running() {
        ui.label("Running: the panel still edits the selected cue.");
    }
}

fn curve_picker(ui: &mut Ui, id: &str, curves: &CurveLibrary, curve: &mut CurveRef) {
    ComboBox::from_id_salt(id)
        .selected_text(format!("curve: {}", curves.name(*curve)))
        .show_ui(ui, |ui| {
            for choice in curves.choices() {
                ui.selectable_value(curve, choice, curves.name(choice));
            }
        });
}

fn curves_section(ui: &mut Ui, curves: &mut CurveLibrary, state: &mut UiState) {
    CollapsingHeader::new("Curves").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for c in &curves.custom {
                if ui
                    .selectable_label(state.editing_curve == Some(c.id), &c.name)
                    .clicked()
                {
                    state.editing_curve = Some(c.id);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("New curve").clicked() {
                state.editing_curve = curves.add().or(state.editing_curve);
            }
            if let Some(id) = state.editing_curve
                && ui.button("Delete").clicked()
            {
                curves.remove(id);
                state.editing_curve = None;
            }
        });
        if let Some(curve) = state.editing_curve.and_then(|id| curves.get_mut(id)) {
            ui.text_edit_singleline(&mut curve.name);
            curve_editor(ui, curve);
            ui.small("Drag points · double-click to add · right-click to remove");
        }
    });
}

fn warp_section(ui: &mut Ui, params: &mut Params) {
    let warp = &mut params.warp;
    CollapsingHeader::new("Deflection")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut warp.zoom, ranges::ZOOM).text("zoom"));
            ui.add(Slider::new(&mut warp.rotation, ranges::ROTATION).text("rotation"));
            ui.add(Slider::new(&mut warp.offset[0], ranges::OFFSET).text("offset x"));
            ui.add(Slider::new(&mut warp.offset[1], ranges::OFFSET).text("offset y"));
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
            ui.checkbox(
                &mut warp.slave_4_to_3,
                "Slave osc 4 to osc 3 (sine/cosine pair)",
            );
            let slaved = warp.slave_4_to_3;
            for (i, osc) in warp.oscillators.iter_mut().enumerate() {
                // A slaved oscillator 4 keeps only its own target, amplitude and envelope.
                let follows = slaved && i == 3;
                CollapsingHeader::new(format!("Oscillator {}", i + 1))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut osc.target, Axis::X, "→ X");
                            ui.selectable_value(&mut osc.target, Axis::Y, "→ Y");
                            ComboBox::from_id_salt(("envelope", i))
                                .selected_text(format!("{:?}", osc.envelope))
                                .show_ui(ui, |ui| {
                                    for e in Envelope::ALL {
                                        ui.selectable_value(&mut osc.envelope, e, format!("{e:?}"));
                                    }
                                });
                        });
                        ui.add(
                            Slider::new(&mut osc.amplitude, ranges::AMPLITUDE).text("amplitude"),
                        );
                        ui.add_enabled_ui(!follows, |ui| oscillator_shape(ui, i, osc));
                    });
            }
        });
}

/// The controls a slaved oscillator 4 takes from oscillator 3.
fn oscillator_shape(ui: &mut Ui, i: usize, osc: &mut Oscillator) {
    ui.horizontal(|ui| {
        ComboBox::from_id_salt(("waveform", i))
            .selected_text(format!("{:?}", osc.waveform))
            .show_ui(ui, |ui| {
                for w in Waveform::ALL {
                    ui.selectable_value(&mut osc.waveform, w, format!("{w:?}"));
                }
            });
        ComboBox::from_id_salt(("input", i))
            .selected_text(format!("by {:?}", osc.input))
            .show_ui(ui, |ui| {
                for input in OscInput::ALL {
                    ui.selectable_value(&mut osc.input, input, format!("by {input:?}"));
                }
            });
        ComboBox::from_id_salt(("sync", i))
            .selected_text(format!("{:?} sync", osc.sync))
            .show_ui(ui, |ui| {
                for sync in OscSync::ALL {
                    ui.selectable_value(&mut osc.sync, sync, format!("{sync:?} sync"));
                }
            });
    });
    ui.add(Slider::new(&mut osc.frequency, ranges::FREQUENCY).text("frequency"));
    ui.add(Slider::new(&mut osc.phase, ranges::PHASE).text("phase"));
    ui.add(Slider::new(&mut osc.phase_speed, ranges::PHASE_SPEED).text("phase speed"));
    ui.add(Slider::new(&mut osc.lfo_rate, ranges::LFO_RATE).text("LFO rate"));
    ui.add(Slider::new(&mut osc.lfo_depth, ranges::LFO_DEPTH).text("LFO depth"));
}

fn colorize_section(ui: &mut Ui, params: &mut Params) {
    let c = &mut params.colorize;
    CollapsingHeader::new("Colorize")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut c.bypass, "Bypass (grayscale)");
            ui.add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"));
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
            ui.add(Slider::new(&mut c.cycle_speed, ranges::CYCLE_SPEED).text("cycle speed"));
            ui.horizontal_wrapped(|ui| {
                for color in &mut c.palette {
                    ui.color_edit_button_rgb(color);
                }
            });
        });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
    let f = &mut params.feedback;
    CollapsingHeader::new("Feedback")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut f.amount, ranges::FEEDBACK_AMOUNT).text("amount"));
            ui.add(Slider::new(&mut f.zoom, ranges::FEEDBACK_ZOOM).text("zoom"));
            ui.add(Slider::new(&mut f.rotation, ranges::FEEDBACK_ROTATION).text("rotation"));
            ui.add(Slider::new(&mut f.offset[0], ranges::FEEDBACK_OFFSET).text("offset x"));
            ui.add(Slider::new(&mut f.offset[1], ranges::FEEDBACK_OFFSET).text("offset y"));
            if ui.button("Clear trails").clicked() {
                actions.clear_feedback = true;
            }
        });
}

fn glow_section(ui: &mut Ui, params: &mut Params) {
    let g = &mut params.glow;
    CollapsingHeader::new("Glow & CRT")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY).text("bloom"));
            ui.add(
                Slider::new(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD)
                    .text("bloom threshold"),
            );
            ui.add(
                Slider::new(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH).text("scanlines"),
            );
            ui.add(
                Slider::new(&mut g.scanline_count, ranges::SCANLINE_COUNT).text("scanline count"),
            );
            ui.add(Slider::new(&mut g.chroma, ranges::CHROMA).text("chroma bleed"));
            ui.add(Slider::new(&mut g.noise, ranges::NOISE).text("noise"));
        });
}
```

- [ ] **Step 2: Drive the app by the canvas clock.** Replace `src/app.rs` with:

```rust
//! The windowed application: owns the GPU context, renderer, parameters and UI. Each
//! screen refresh draws the canvas frames that are due at the program frame rate, then
//! shows the newest one with the UI on top.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::blend::FrameParams;
use crate::canvas::{self, CanvasChoice};
use crate::capture::CaptureMode;
use crate::capture::recorder::Recorder;
use crate::clock::{CanvasClock, Pacing};
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(mut state) => {
                // Startup took a while; the first canvas frame is due now.
                state.clock.reanchor(state.seconds());
                self.state = Some(state);
            }
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let response = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                state.stop_recording(); // finish the file before exiting
                event_loop.exit();
            }
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
            // Space triggers a transition unless egui is using the keyboard (e.g. a text field).
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && event.logical_key == Key::Named(NamedKey::Space)
                    && !response.consumed
                    && !state.egui_ctx.egui_wants_keyboard_input() =>
            {
                state.motion.trigger();
            }
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_side: u32,
    composite_format: wgpu::TextureFormat,
    renderer: Renderer,
    preview: PreviewView,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    motion: Motion,
    recorder: Option<Recorder>,
    ui: UiState,
    /// Paces canvas frames at the program frame rate.
    clock: CanvasClock,
    /// Wall-clock origin for the canvas clock.
    epoch: Instant,
    /// Animation time of the newest canvas frame.
    time: f64,
    /// What the newest canvas frame was drawn with; refreshes with no new canvas frame
    /// composite it again.
    frame_params: FrameParams,
    /// When the previous screen refresh started, for the display rate readout.
    last_refresh: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .with_context(|| {
                format!(
                    "failed to create window surface for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl"
                )
            })?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;
        let max_texture_side = device.limits().max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects). Where the adapter allows an
        // sRGB view of it, the composite pass draws through that view; otherwise (e.g. GL)
        // it draws into the plain format and encodes sRGB in the shader.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        let srgb_view_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let composite_format = if srgb_view_ok { srgb_format } else { format };
        // Fifo (vsync) presents every refresh. RASTERWARP_UNCAPPED=1 opts into Mailbox
        // (no vsync wait, no tearing); canvas frames are paced by the frame rate either way.
        let uncapped = std::env::var("RASTERWARP_UNCAPPED").is_ok_and(|v| v == "1");
        let present_mode = if uncapped && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = if srgb_view_ok {
            vec![srgb_format]
        } else {
            vec![]
        };
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {composite_format:?}), {present_mode:?}");

        let mut ui = UiState {
            frame_ms: 16.7,
            show_preview: true,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match load_fitting(path, max_texture_side) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
        let renderer = Renderer::new(&device, &queue, composite_format, canvas::DEFAULT, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview =
            PreviewView::new(&device, &queue, &mut egui_renderer, canvas::DEFAULT, &image);
        let motion = Motion::new(Params::default());
        let frame_params = motion.frame();
        let epoch = Instant::now();

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
            egui_ctx,
            egui_state,
            egui_renderer,
            motion,
            recorder: None,
            clock: CanvasClock::new(ui.rate, 0.0),
            ui,
            epoch,
            time: 0.0,
            frame_params,
            last_refresh: epoch,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.preview.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    /// Wall-clock seconds since the app started, for the canvas clock.
    fn seconds(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn start_recording(&mut self) {
        self.ui.capture.saved = None;
        match Recorder::start(
            &self.device,
            &self.ui.capture.settings,
            self.renderer.size(),
            self.clock.rate(),
        ) {
            Ok(recorder) => {
                // Opening the encoder stalls; that must not make canvas frames late.
                self.clock.reanchor(self.seconds());
                self.ui.capture.error = None;
                self.ui.capture.status = Some(recorder.status());
                self.recorder = Some(recorder);
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("{err:#}"));
            }
        }
    }

    /// Finishes the current recording, if any, and reports how it went.
    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        self.ui.capture.status = None;
        let result = recorder.finish(&self.device);
        // Flushing the encoder stalls; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
        match result {
            Ok(status) => {
                self.ui.capture.saved = Some(format!(
                    "Saved {} ({:.1} s, {} frames at {}, {} dropped)",
                    status.path.display(),
                    status.seconds,
                    status.frames,
                    self.clock.rate().label(),
                    status.dropped
                ));
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("Recording stopped: {err:#}"));
            }
        }
    }

    fn choose_capture_folder(&mut self) {
        let folder = &mut self.ui.capture.settings.folder;
        let mut dialog = rfd::FileDialog::new().set_title("Capture folder");
        if let Ok(start) = std::path::absolute(&*folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            *folder = picked;
        }
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// Draws one canvas frame: advances animation by one frame period (unless paused),
    /// draws the canvas and the preview, and records the frame if a recording is running.
    fn draw_canvas_frame(&mut self) {
        let paused = self.ui.paused;
        if !paused {
            let period = self.clock.rate().period();
            self.time += period;
            self.motion.advance(period as f32);
        }
        self.frame_params = self.motion.frame();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.render_canvas(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.frame_params,
            self.time as f32,
        );
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
        let mut capture_failed = false;
        // A paused frame repeats the last one, so it isn't recorded.
        if let Some(recorder) = &mut self.recorder
            && !paused
        {
            let (device, queue, renderer) = (&self.device, &self.queue, &self.renderer);
            let (frame_params, time) = (&self.frame_params, self.time as f32);
            let captured = recorder.capture(device, &mut encoder, |encoder, view| {
                renderer.composite_capture(device, queue, encoder, frame_params, time, view)
            });
            if let Err(err) = captured {
                log::warn!("{err:#}");
                capture_failed = true;
            }
        }
        self.queue.submit([encoder.finish()]);
        if let Some(recorder) = &mut self.recorder {
            recorder.after_submit();
            self.ui.capture.status = Some(recorder.status());
            if capture_failed || recorder.finished_recording() {
                self.stop_recording();
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_refresh).as_secs_f32();
        self.last_refresh = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;

        if self.config.width == 0 || self.config.height == 0 {
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        if let Some(recorder) = &mut self.recorder
            && let Err(err) = recorder.pump(&self.device)
        {
            log::warn!("{err:#}");
            self.stop_recording();
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        self.ui.late = self.clock.late();
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        if actions.choose_folder {
            self.choose_capture_folder();
        }
        if actions.stop_recording {
            self.stop_recording();
        }
        if let Some(size) = actions.apply_canvas
            && self.recorder.is_none()
        {
            self.renderer.resize(&self.device, size);
            self.preview
                .resize(&self.device, &mut self.egui_renderer, size);
            self.ui.canvas.current = size;
        }
        if self.ui.rate != self.clock.rate() && self.recorder.is_none() {
            self.clock.set_rate(self.ui.rate, self.seconds());
        }
        if actions.start_recording && self.recorder.is_none() {
            self.start_recording();
        }
        if actions.clear_feedback {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
            self.queue.submit([encoder.finish()]);
        }

        let offline = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.mode() == CaptureMode::Offline);
        let pacing = if offline {
            Pacing::Offline
        } else {
            Pacing::RealTime
        };
        for _ in 0..self.clock.due(self.seconds(), pacing) {
            self.draw_canvas_frame();
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.composite(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.frame_params,
            self.time as f32,
            &srgb_view,
            output_size,
        );
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}

/// Loads an image and checks it fits in a GPU texture.
fn load_fitting(path: &Path, max_side: u32) -> anyhow::Result<GrayImage> {
    let image = source::load_image(path)?;
    source::ensure_fits(&image, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}
```

- [ ] **Step 3: Run everything.** Run: `cargo test`. Expected: `119 passed` for the unit tests, `4 passed` in `tests/capture.rs`, `7 passed` in `tests/smoke.rs`.

- [ ] **Step 4: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 5: Manual check (release).** Run: `cargo run --release` on the 60 Hz display, then check each item. ffprobe is in `C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared\bin`.
  - [ ] The header reads "60 fps · display 60 Hz · 0 late". **Frame rate** lists 23.976, 24, 25, 29.97, 30, 50, 59.94 and 60 fps.
  - [ ] Choose **24 fps**. The header shows "24 fps". On screen each canvas frame shows for 2 or 3 refreshes, the panel stays responsive, and the late count stays 0.
  - [ ] Record HEVC **Real-time** for about 4 s, then stop.
    - While recording, **Frame rate** is greyed out.
    - The saved line reads "… frames at 24 fps, 0 dropped".
    - `ffprobe -v error -count_packets -show_entries stream=r_frame_rate,avg_frame_rate,nb_read_packets -of default=nw=1 <file>` reports `24/1` and the panel's frame count.
    - `ffprobe -v error -show_entries packet=pts -of csv=p=0 <file>` lists timestamps that are all exactly one frame apart.
  - [ ] Choose **23.976 fps**, **FFV1**, **Offline** and **Stop after 2.0 s**, then press **Record**. It stops by itself. The saved line reads "2.0 s, 48 frames at 23.976 fps, 0 dropped", and ffprobe reports `24000/1001` and 48 packets.
  - [ ] **Pause**: the picture freezes, but moving a slider still changes it. Unpause: animation carries on without a jump, and the late count doesn't grow.
  - [ ] Drag the window for a second or two. Afterwards the late count has grown, and animation carries on from where it was rather than jumping ahead.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/ui.rs
git commit -m "feat: pace the canvas by the program frame rate" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done when

- `cargo test` passes (119 unit, 4 capture and 7 smoke tests). `cargo clippy --all-targets` is clean and `cargo fmt` makes no changes.
- The manual checklist in Task 4 passes on the 60 Hz display.
