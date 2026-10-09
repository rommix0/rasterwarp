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
        self.held = None;
        self.info = info;
    }

    /// Switches to the camera `choice`. Its picture appears with its first frame.
    pub fn open_camera(&mut self, choice: &CameraChoice, canvas: (u32, u32), budget: &Budget) {
        self.loading = None;
        // Dropping the old camera gives its buffer's memory back at once, so the new
        // buffer can use it.
        self.content = Content::Still;
        self.format = None;
        self.held = None;
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
        self.held = None;
        self.playback = [Playback::default(); 2];
        self.modes = [None; 2];
    }

    /// Takes over a clip that finished loading, and makes frames passes for a camera
    /// once it has a picture (or reopened in another format), so what was showing stays
    /// until then. Returns the clip's path when one took over, or why it failed to load.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderers: [&mut Renderer; 2],
    ) -> Option<Result<PathBuf>> {
        if let Content::Camera(camera) = &self.content
            && camera.has_frames()
            && let Some(format) = camera.format()
            && self.format != Some(format)
        {
            let (w, h) = format.size;
            let info = format!(
                "{}: {w}×{h} at {} fps",
                camera.name,
                describe_fps(camera.fps())
            );
            self.make_passes(device, queue, renderers, format);
            self.info = info;
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
        let Some(format) = self.format else {
            return;
        };
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
                // A camera that reopened in another size is fed again once `poll` has
                // remade the passes for it.
                if camera.format() != Some(format) {
                    return;
                }
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
                        let pixels = if small { &frame.small } else { &frame.full };
                        // One from a reopening between the check above and now.
                        if pixels.len() as u64 != format.bytes_at(pass.size()) {
                            continue;
                        }
                        pass.upload(queue, k, pixels);
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
