//! The canvas: the internal render resolution, chosen from presets or typed in.

/// The canvas size at startup.
pub const DEFAULT: (u32, u32) = (1920, 1080);
/// The smallest allowed canvas side, in pixels.
pub const MIN_SIDE: u32 = 64;

pub const PRESETS: [(&str, (u32, u32)); 6] = [
    ("480p 4:3", (640, 480)),
    ("480p 16:9", (854, 480)),
    ("720p", (1280, 720)),
    ("1080p", (1920, 1080)),
    ("1440p", (2560, 1440)),
    ("2160p (4K)", (3840, 2160)),
];

/// Rounds each side up to an even number and clamps it to [`MIN_SIDE`, `max_side`].
pub fn sanitize(size: (u32, u32), max_side: u32) -> (u32, u32) {
    let max_even = max_side & !1;
    let side = |v: u32| (v.saturating_add(1) & !1).clamp(MIN_SIDE, max_even);
    (side(size.0), side(size.1))
}

/// The panel's canvas controls.
#[derive(Clone, Debug)]
pub struct CanvasChoice {
    /// An index into [`PRESETS`], or `PRESETS.len()` for a custom size.
    pub choice: usize,
    pub custom: (u32, u32),
    /// The size the renderer is using now.
    pub current: (u32, u32),
    /// The GPU's largest texture side.
    pub max_side: u32,
}

impl CanvasChoice {
    pub const CUSTOM: usize = PRESETS.len();

    pub fn new(current: (u32, u32), max_side: u32) -> Self {
        let choice = PRESETS
            .iter()
            .position(|(_, size)| *size == current)
            .unwrap_or(Self::CUSTOM);
        Self {
            choice,
            custom: current,
            current,
            max_side,
        }
    }

    /// The size that Apply would switch to.
    pub fn requested(&self) -> (u32, u32) {
        let size = PRESETS
            .get(self.choice)
            .map_or(self.custom, |(_, size)| *size);
        sanitize(size, self.max_side)
    }
}

impl Default for CanvasChoice {
    fn default() -> Self {
        Self::new(DEFAULT, 8192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_rounded_up_to_even() {
        assert_eq!(sanitize((1001, 721), 8192), (1002, 722));
        assert_eq!(sanitize((1280, 720), 8192), (1280, 720));
    }

    #[test]
    fn sizes_are_clamped_to_the_limits() {
        assert_eq!(sanitize((10, 0), 8192), (64, 64));
        assert_eq!(sanitize((9000, 9000), 8192), (8192, 8192));
        assert_eq!(sanitize((9000, 100), 8191), (8190, 100));
    }

    #[test]
    fn current_size_selects_its_preset() {
        assert_eq!(CanvasChoice::new((1280, 720), 8192).choice, 2);
        assert_eq!(
            CanvasChoice::new((1000, 500), 8192).choice,
            CanvasChoice::CUSTOM
        );
    }

    #[test]
    fn requested_size_comes_from_the_preset_or_the_custom_fields() {
        let mut c = CanvasChoice::new(DEFAULT, 4096);
        c.choice = 5;
        assert_eq!(c.requested(), (3840, 2160));
        c.choice = CanvasChoice::CUSTOM;
        c.custom = (5001, 333);
        assert_eq!(c.requested(), (4096, 334));
    }
}
