//! The off-air preview: a second, small renderer with its own trails, drawn into a
//! texture that the panel shows in the bottom-right corner of the window.

use crate::gpu::RenderTarget;
use crate::motion::{Preview, PreviewSource};
use crate::passes::Renderer;
use crate::source::GrayImage;

/// Preview width in pixels; the height follows the canvas aspect ratio.
pub const WIDTH: u32 = 480;
/// The preview texture's format. egui treats texture bytes as already sRGB-encoded, so
/// this is a plain format and the composite shader does the encoding.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The preview size for a canvas: 480 wide, the canvas's aspect ratio, and an even
/// height of at least 64.
pub fn size_for(canvas: (u32, u32)) -> (u32, u32) {
    let height = (WIDTH as f32 * canvas.1 as f32 / canvas.0 as f32 / 2.0).round() as u32 * 2;
    (WIDTH, height.max(64))
}

pub struct PreviewView {
    renderer: Renderer,
    target: RenderTarget,
    texture_id: egui::TextureId,
    /// What the trails currently belong to; a change clears them.
    shown: Option<PreviewSource>,
}

impl PreviewView {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
        canvas: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let size = size_for(canvas);
        let target = RenderTarget::new(device, "preview", size.0, size.1, FORMAT);
        let texture_id =
            egui_renderer.register_native_texture(device, &target.view, wgpu::FilterMode::Linear);
        Self {
            renderer: Renderer::new(device, queue, FORMAT, size, image),
            target,
            texture_id,
            shown: None,
        }
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    /// The preview texture's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.target.texture.width(), self.target.texture.height())
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.renderer.set_source(device, queue, image);
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.renderer.clear_feedback(encoder);
    }

    /// Forgets what was shown, so the trails are cleared when the preview appears again.
    pub fn hide(&mut self) {
        self.shown = None;
    }

    /// Records the preview frame into `encoder`, clearing the trails first when the
    /// preview has switched to a different bank or cue.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        preview: &Preview,
        time: f32,
    ) {
        if self.shown != Some(preview.source) {
            self.renderer.clear_feedback(encoder);
            self.shown = Some(preview.source);
        }
        let size = self.size();
        self.renderer.render(
            device,
            queue,
            encoder,
            &preview.frame,
            time,
            &self.target.view,
            size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_the_canvas_aspect_ratio() {
        assert_eq!(size_for((1920, 1080)), (480, 270));
        assert_eq!(size_for((640, 480)), (480, 360));
        assert_eq!(size_for((854, 480)), (480, 270));
    }

    #[test]
    fn preview_height_is_even_and_at_least_64() {
        assert_eq!(size_for((1000, 333)).1 % 2, 0);
        assert_eq!(size_for((4000, 64)), (480, 64));
    }
}
