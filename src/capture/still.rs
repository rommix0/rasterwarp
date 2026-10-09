//! Stills: one canvas frame saved as a PNG next to the recordings.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::readback::Readback;
use super::unused_path;

/// `rasterwarp-YYYYMMDD-HHMMSS.png` for a still taken at `now` (local time).
pub fn file_name(now: chrono::NaiveDateTime) -> String {
    format!("rasterwarp-{}.png", now.format("%Y%m%d-%H%M%S"))
}

/// RGB from RGBA, for an opaque still: the alpha channel is dropped.
pub fn rgb(rgba: &[u8]) -> Vec<u8> {
    let (pixels, _) = rgba.as_chunks::<4>();
    pixels.iter().flat_map(|px| [px[0], px[1], px[2]]).collect()
}

/// Draws one frame with `draw` into a texture of `size` and reads it back as sRGB
/// RGBA bytes, waiting for the GPU.
pub fn grab(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: (u32, u32),
    draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
) -> Result<Vec<u8>> {
    let mut readback = Readback::new(device, size);
    let mut encoder = device.create_command_encoder(&Default::default());
    draw(&mut encoder, readback.view());
    readback.copy(&mut encoder, 0);
    queue.submit([encoder.finish()]);
    readback.after_submit();
    match readback.collect(device, true, Vec::new)?.pop() {
        Some(frame) => Ok(frame.rgba),
        None => bail!("the still did not come back from the GPU"),
    }
}

/// Saves `rgba` (`size` pixels) as a PNG in `folder`, named for `now`, without replacing
/// an earlier still: RGBA with `alpha`, otherwise RGB. Returns where it went.
pub fn save(
    folder: &Path,
    now: chrono::NaiveDateTime,
    size: (u32, u32),
    rgba: &[u8],
    alpha: bool,
) -> Result<PathBuf> {
    std::fs::create_dir_all(folder)
        .with_context(|| format!("could not create {}", folder.display()))?;
    let path = unused_path(folder, &file_name(now));
    let saved = if alpha {
        image::save_buffer(&path, rgba, size.0, size.1, image::ExtendedColorType::Rgba8)
    } else {
        image::save_buffer(
            &path,
            &rgb(rgba),
            size.0,
            size.1,
            image::ExtendedColorType::Rgb8,
        )
    };
    saved.with_context(|| format!("could not save {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_nine() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_opt(9, 5, 3)
            .unwrap()
    }

    #[test]
    fn stills_are_named_after_the_time_they_were_taken() {
        assert_eq!(file_name(at_nine()), "rasterwarp-20261007-090503.png");
    }

    #[test]
    fn the_alpha_channel_is_dropped() {
        assert_eq!(rgb(&[1, 2, 3, 255, 4, 5, 6, 0]), [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn stills_are_saved_as_pngs_without_replacing_earlier_ones() {
        let folder = crate::save::temp_dir("stills").join("captures");
        // Two 2×1 frames: red then green, and blue then white.
        let first = [255, 0, 0, 255, 0, 255, 0, 255];
        let second = [0, 0, 255, 255, 255, 255, 255, 255];
        let a = save(&folder, at_nine(), (2, 1), &first, false).unwrap();
        let b = save(&folder, at_nine(), (2, 1), &second, false).unwrap();
        assert_eq!(a, folder.join("rasterwarp-20261007-090503.png"));
        assert_eq!(b, folder.join("rasterwarp-20261007-090503-2.png"));
        let image = image::open(&a).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.as_raw(), &[255, 0, 0, 0, 255, 0]);
        let image = image::open(&b).unwrap().to_rgb8();
        assert_eq!(image.as_raw(), &[0, 0, 255, 255, 255, 255]);
        assert!(!image::open(&a).unwrap().color().has_alpha());
    }

    #[test]
    fn transparent_stills_keep_their_alpha() {
        let folder = crate::save::temp_dir("alpha-stills").join("captures");
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0];
        let path = save(&folder, at_nine(), (2, 1), &rgba, true).unwrap();
        let image = image::open(&path).unwrap();
        assert!(image.color().has_alpha());
        assert_eq!(image.to_rgba8().as_raw(), &rgba);
    }
}
