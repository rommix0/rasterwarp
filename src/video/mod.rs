//! Video inputs: clips decoded into memory and live cameras, and the playhead that
//! picks their frames.

pub mod camera;
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
