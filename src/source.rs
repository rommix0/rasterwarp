//! Source artwork (image files and the built-in test card, as 8-bit grayscale) and the
//! color background image that keyed levels show.

use std::path::Path;

use anyhow::{Context, Result};

/// 8-bit grayscale artwork, row-major, one byte per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl GrayImage {
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

/// Converts any decoded image to grayscale (luma).
pub fn from_dynamic(image: image::DynamicImage) -> GrayImage {
    let luma = image.to_luma8();
    GrayImage {
        width: luma.width(),
        height: luma.height(),
        pixels: luma.into_raw(),
    }
}

pub fn load_image(path: &Path) -> Result<GrayImage> {
    let image = image::open(path).with_context(|| format!("could not load {}", path.display()))?;
    Ok(from_dynamic(image))
}

/// Errors if the image is too large to upload as a single GPU texture.
pub fn ensure_fits(image: &GrayImage, max_side: u32) -> Result<()> {
    ensure_size_fits(image.width, image.height, max_side)
}

/// Errors if a `width`×`height` image is too large to upload as a single GPU texture.
pub fn ensure_size_fits(width: u32, height: u32, max_side: u32) -> Result<()> {
    if width > max_side || height > max_side {
        anyhow::bail!(
            "image is {width}×{height}, larger than the GPU texture limit of {max_side}×{max_side}"
        );
    }
    Ok(())
}

/// A color image (the keying background), 8-bit sRGB RGBA, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ColorImage {
    /// A single black pixel: what keyed levels show without a background image.
    pub fn black() -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
        }
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

pub fn load_color(path: &Path) -> Result<ColorImage> {
    let image = image::open(path).with_context(|| format!("could not load {}", path.display()))?;
    let rgba = image.to_rgba8();
    Ok(ColorImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

/// Uploads a color image as an `Rgba8UnormSrgb` texture, so sampling returns linear light.
pub fn upload_color(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &ColorImage,
) -> wgpu::Texture {
    let size = wgpu::Extent3d {
        width: image.width,
        height: image.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("background"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width * 4),
            rows_per_image: None,
        },
        size,
    );
    texture
}

/// Procedural test card with many gray levels so every colorizer level gets used:
/// concentric rings, stepped bars, a hollow block, and a gradient strip.
pub fn test_card(width: u32, height: u32) -> GrayImage {
    let (w, h) = (width as f32, height as f32);
    let ring_levels = [255u8, 0, 200, 0, 145, 0, 90];
    let mut pixels = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (u, v) = (fx / w, fy / h);
            let ring_radius = 0.42 * h;
            let d = ((fx - 0.3 * w).powi(2) + (fy - 0.5 * h).powi(2)).sqrt() / ring_radius;
            let level = if d < 1.0 {
                ring_levels[(d * ring_levels.len() as f32) as usize]
            } else if (0.6..0.95).contains(&u) && (0.08..0.45).contains(&v) {
                let bar = ((u - 0.6) / 0.35 * 8.0) as u8;
                bar.min(7) * 32 + 31
            } else if (0.6..0.72).contains(&u) && (0.5..0.68).contains(&v) {
                let hole = (0.63..0.69).contains(&u) && (0.545..0.635).contains(&v);
                if hole { 0 } else { 220 }
            } else if (0.6..0.95).contains(&u) && (0.75..0.9).contains(&v) {
                ((u - 0.6) / 0.35 * 255.0).round() as u8
            } else {
                0
            };
            pixels[(y * width + x) as usize] = level;
        }
    }
    GrayImage {
        width,
        height,
        pixels,
    }
}

/// Uploads grayscale artwork as an `R8Unorm` texture.
pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) -> wgpu::Texture {
    let size = wgpu::Extent3d {
        width: image.width,
        height: image.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("source"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width),
            rows_per_image: None,
        },
        size,
    );
    texture
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_card_has_requested_size() {
        let card = test_card(320, 180);
        assert_eq!((card.width, card.height), (320, 180));
        assert_eq!(card.pixels.len(), 320 * 180);
    }

    #[test]
    fn test_card_spans_many_gray_levels() {
        let card = test_card(320, 180);
        let levels: BTreeSet<u8> = card.pixels.iter().copied().collect();
        assert!(levels.len() >= 8, "only {} distinct levels", levels.len());
        assert_eq!(levels.first(), Some(&0));
        assert_eq!(levels.last(), Some(&255));
    }

    #[test]
    fn converts_rgb_to_luma() {
        let mut rgb = image::RgbImage::new(2, 1);
        rgb.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        rgb.put_pixel(1, 0, image::Rgb([255, 255, 255]));
        let gray = from_dynamic(image::DynamicImage::ImageRgb8(rgb));
        assert_eq!((gray.width, gray.height), (2, 1));
        // Rec. 709 luma: pure red is about 21% brightness.
        assert_eq!(gray.pixels, vec![54, 255]);
    }

    #[test]
    fn missing_file_is_an_error() {
        let err = load_image(Path::new("does-not-exist.png")).unwrap_err();
        assert!(err.to_string().contains("does-not-exist.png"));
        let err = load_color(Path::new("no-background.png")).unwrap_err();
        assert!(err.to_string().contains("no-background.png"));
    }

    #[test]
    fn loads_a_color_image_as_rgba() {
        let path = std::env::temp_dir().join(format!("rasterwarp-bg-{}.png", std::process::id()));
        let mut rgb = image::RgbImage::new(3, 2);
        rgb.put_pixel(2, 1, image::Rgb([200, 100, 50]));
        rgb.save(&path).expect("write test image");
        let loaded = load_color(&path).expect("load test image");
        let _ = std::fs::remove_file(&path);
        assert_eq!((loaded.width, loaded.height), (3, 2));
        assert_eq!(&loaded.pixels[20..24], &[200, 100, 50, 255]);
        assert_eq!(loaded.aspect(), 1.5);
        assert_eq!(ColorImage::black().pixels, vec![0, 0, 0, 255]);
    }

    #[test]
    fn ensure_fits_accepts_images_within_limit() {
        assert!(ensure_fits(&test_card(64, 32), 64).is_ok());
    }

    #[test]
    fn ensure_fits_rejects_oversized_images() {
        let err = ensure_fits(&test_card(65, 32), 64).unwrap_err();
        assert!(err.to_string().contains("65×32"));
        assert!(ensure_size_fits(64, 65, 64).is_err());
    }
}
