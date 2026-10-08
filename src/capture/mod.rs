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
