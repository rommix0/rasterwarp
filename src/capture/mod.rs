//! Video capture: recording the clean canvas output to a file.

pub mod encode;
pub mod readback;
pub mod recorder;

use encode::{FPS, VideoFormat};

/// Animation time per frame while recording offline.
pub const OFFLINE_STEP: f32 = 1.0 / FPS as f32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture at 60 fps from the live output; frames the encoder can't keep up with are
    /// dropped and counted.
    #[default]
    RealTime,
    /// Advance animation time by exactly 1/60 s per rendered frame and capture every
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

/// Decides which rendered frames are recorded and their timestamps (in frames at 60 fps).
#[derive(Clone, Debug)]
pub struct FrameClock {
    mode: CaptureMode,
    /// Animation time when the recording started.
    start: f64,
    /// Animation time of the last recorded frame.
    last: f64,
    /// The next timestamp to hand out; also the recording's length in frames.
    next: i64,
}

impl FrameClock {
    pub fn new(mode: CaptureMode, start: f64) -> Self {
        Self {
            mode,
            start,
            last: start,
            next: 0,
        }
    }

    /// The timestamp for a frame rendered at animation time `time`, or `None` to skip it.
    /// Real-time mode records a frame whenever `(time − start) · 60`, rounded to the nearest
    /// frame, moves on (rounding keeps a 60 Hz display's jittery frame times inside their own
    /// frame). Offline mode records every frame whose time has moved on (so pausing records
    /// nothing).
    pub fn frame_due(&mut self, time: f64) -> Option<i64> {
        let pts = match self.mode {
            CaptureMode::Offline => {
                if self.next > 0 && time <= self.last {
                    return None;
                }
                self.next
            }
            CaptureMode::RealTime => {
                let n = ((time - self.start) * f64::from(FPS) + 0.5).floor() as i64;
                if n < self.next {
                    return None;
                }
                n
            }
        };
        self.next = pts + 1;
        self.last = time;
        Some(pts)
    }

    /// The recording's length in frames so far.
    pub fn frames(&self) -> i64 {
        self.next
    }

    /// The recording's length in seconds so far.
    pub fn seconds(&self) -> f64 {
        self.next as f64 / f64::from(FPS)
    }

    /// Whether a recording limited to `seconds` (0 = no limit) is long enough. Counted
    /// in whole frames, so 10/60 s stops after exactly 10 frames.
    pub fn reached(&self, seconds: f32) -> bool {
        seconds > 0.0 && self.next >= (f64::from(seconds) * f64::from(FPS)).round() as i64
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
    fn real_time_capture_decimates_a_fast_display_to_60_fps() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        let captured: Vec<i64> = (0..143)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 144.0))
            .collect();
        assert_eq!(captured, (0..60).collect::<Vec<_>>());
        assert!((clock.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn real_time_capture_keeps_every_frame_of_a_jittery_60_hz_display() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        // ±0.3 ms of timestamp noise around exact 60 Hz frame times.
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        let captured: Vec<i64> = (0..600)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 60.0 + jitter[i as usize % 5]))
            .collect();
        assert_eq!(captured, (0..600).collect::<Vec<_>>());
    }

    #[test]
    fn real_time_capture_leaves_gaps_when_rendering_is_slow() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        let captured: Vec<i64> = (0..4)
            .filter_map(|i| clock.frame_due(f64::from(i) / 30.0))
            .collect();
        assert_eq!(captured, [0, 2, 4, 6]);
    }

    #[test]
    fn paused_time_records_nothing_new() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        assert_eq!(clock.frame_due(0.5), Some(30));
        assert_eq!(clock.frame_due(0.5), None);
    }

    #[test]
    fn offline_capture_records_every_frame() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 3.0);
        let captured: Vec<i64> = (0..5)
            .filter_map(|i| clock.frame_due(3.0 + f64::from(i) * 0.25))
            .collect();
        assert_eq!(captured, [0, 1, 2, 3, 4]);
        assert!((clock.seconds() - 5.0 / 60.0).abs() < 1e-9);
        assert_eq!(clock.frame_due(4.0), None, "paused: time hasn't moved");
    }

    #[test]
    fn stop_after_counts_whole_frames() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 0.0);
        for i in 0..9 {
            clock.frame_due(f64::from(i));
        }
        assert!(!clock.reached(10.0 / 60.0));
        clock.frame_due(9.0);
        assert!(clock.reached(10.0 / 60.0));
        assert!(!clock.reached(0.0), "0 means no limit");
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
