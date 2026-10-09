//! The app's two inputs (the source and the background) while it runs: whether each
//! shows an image, a clip or a camera; loading clips and opening cameras; and feeding
//! their frames to the renderers every canvas frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};

use crate::blend::VideoFrame;
use crate::params::{PlayMode, Role, Slit, ranges};
use crate::passes::Renderer;
use crate::passes::frames::FramesPass;
use crate::session::CameraChoice;
use crate::source::GrayImage;
use crate::video::camera::{Camera, Status};
use crate::video::clip::{Clip, Loading};
use crate::video::playhead::{
    self, Arrival, Playback, Sample, Trail, camera_sample, clip_frame, clip_sample, loop_span,
    max_layers,
};
use crate::video::{Budget, Format, Frame, Pixels, Reservation, describe_bytes};

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

/// A camera's last frames, held to play as a clip.
#[derive(Debug)]
struct Looped {
    frames: Vec<Arc<Frame>>,
    /// The camera's measured frame rate while they arrived.
    fps: f64,
    /// The loop's length.
    seconds: f32,
    /// The memory the frames take from the budget. They're the camera's too, but outlive
    /// its buffer, so they're counted again.
    _memory: Reservation,
}

impl Looped {
    /// Holds `frames` of `frame_bytes` each, if the budget has room.
    fn hold(
        frames: Vec<Arc<Frame>>,
        fps: f64,
        seconds: f32,
        frame_bytes: u64,
        budget: &Budget,
    ) -> Result<Self> {
        let needed = frame_bytes.saturating_mul(frames.len() as u64);
        let Some(memory) = budget.take(needed) else {
            bail!(
                "A {seconds:.1} s loop needs {} of memory and only {} is free; shorten the \
                 delay or the camera buffer",
                describe_bytes(needed),
                describe_bytes(budget.available())
            );
        };
        Ok(Self {
            frames,
            fps,
            seconds,
            _memory: memory,
        })
    }

    fn count(&self) -> u32 {
        self.frames.len() as u32
    }
}

/// What virtual indices meant when a view's frame ring was filled.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Filled {
    /// A live camera's frame numbers.
    Live,
    /// Frames of a clip, or of loop number `take`, played in `mode`.
    Frames { take: u64, mode: PlayMode },
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
    /// Where each view's clip playhead has been, for slit-scan.
    trails: [Trail; 2],
    /// What each view's frame ring was filled for.
    filled: [Option<Filled>; 2],
    /// A camera's loop, playing instead of the live picture.
    looped: Option<Looped>,
    /// Loops taken so far: each one's frames are new.
    takes: u64,
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
            trails: Default::default(),
            filled: [None; 2],
            looped: None,
            takes: 0,
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
        let fps = match (&self.content, &self.looped) {
            (Content::Still, _) => return None,
            (Content::Clip(clip), _) => clip.fps,
            (Content::Camera(_), Some(looped)) => looped.fps,
            (Content::Camera(camera), None) => playhead::camera_fps(&camera.arrivals()),
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
        self.looped = None;
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
        self.looped = None;
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
        self.trails = Default::default();
        self.filled = [None; 2];
        // A camera that reopened in another format can't play its old frames.
        self.looped = None;
    }

    /// How long the camera's loop is, while it loops.
    pub fn looping(&self) -> Option<f32> {
        self.looped.as_ref().map(|l| l.seconds)
    }

    /// Loops the camera's last `v.delay` seconds (up to where a paused camera stopped),
    /// replacing any loop. It plays like a clip from the frame the delay was showing, so
    /// the picture carries on, and slit-scan carries on from the live one. `time` is the
    /// animation time.
    pub fn grab_loop(&mut self, v: &VideoFrame, time: f64, budget: &Budget) -> Result<()> {
        let Content::Camera(camera) = &self.content else {
            bail!("Only a camera can loop");
        };
        let Some(format) = self.format else {
            bail!("The camera has no picture to loop yet");
        };
        // Its memory goes back first, so the new loop can use it.
        self.looped = None;
        let arrivals = camera.arrivals();
        let shown = self.shown(&arrivals);
        let Some((first, last, start)) = loop_span(shown, f64::from(v.delay)) else {
            bail!("Set a delay to choose the loop length");
        };
        let held = &shown[shown.partition_point(|a| a.seq < first)..];
        let fps = playhead::camera_fps(held);
        let seconds = (held[held.len() - 1].time - held[0].time) as f32;
        let frames = camera
            .frames(first, last)
            .ok_or_else(|| anyhow!("The camera's buffer moved on; try again"))?;
        self.looped = Some(Looped::hold(
            frames,
            fps,
            seconds.min(v.delay),
            format.frame_bytes(),
            budget,
        )?);
        self.takes += 1;
        self.held = None;
        let keep = f64::from(*ranges::SLIT_DEPTH.end());
        self.playback = [Playback::continuing(start); 2];
        self.trails = std::array::from_fn(|_| Trail::approaching(time, start, fps, keep));
        Ok(())
    }

    /// Goes back to the live camera, letting the loop's frames go.
    pub fn release_loop(&mut self) {
        self.looped = None;
        self.held = None;
    }

    /// The camera frames on show: all of them, or up to where a paused camera stopped.
    fn shown<'a>(&self, arrivals: &'a [Arrival]) -> &'a [Arrival] {
        match self.held {
            Some(time) => &arrivals[..arrivals.partition_point(|a| a.time <= time)],
            None => arrivals,
        }
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
        let (main, preview) = (View::Main.index(), View::Preview.index());
        self.playback[preview] = self.playback[main];
        self.trails[preview] = self.trails[main].clone();
    }

    /// Draws this canvas frame's picture into `renderer` (the canvas's or the
    /// preview's) for playback settings `v` at animation time `time` (seconds, standing
    /// still while paused). A paused camera holds its picture.
    #[allow(clippy::too_many_arguments)]
    pub fn feed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &mut Renderer,
        view: View,
        v: &VideoFrame,
        time: f64,
        paused: bool,
    ) {
        let Some(format) = self.format else {
            return;
        };
        let Some(pass) = renderer.frames_mut(self.role) else {
            return;
        };
        let small = view == View::Preview;
        let i = view.index();
        let (filled, playback, trail) = (
            &mut self.filled[i],
            &mut self.playback[i],
            &mut self.trails[i],
        );
        let mut sample = match (&self.content, &self.looped) {
            (Content::Still, _) => return,
            (Content::Clip(clip), _) => {
                let played = Played {
                    take: 0,
                    count: clip.count(),
                    fps: clip.fps,
                };
                let frame = |f: u32| &clip.frames[f as usize];
                played.feed(
                    device, queue, pass, filled, playback, trail, v, time, small, frame,
                )
            }
            (Content::Camera(_), Some(looped)) => {
                let played = Played {
                    take: self.takes,
                    count: looped.count(),
                    fps: looped.fps,
                };
                let frame = |f: u32| &*looped.frames[f as usize];
                played.feed(
                    device, queue, pass, filled, playback, trail, v, time, small, frame,
                )
            }
            (Content::Camera(camera), None) => {
                // A camera that reopened in another size is fed again once `poll` has
                // remade the passes for it.
                if camera.format() != Some(format) {
                    return;
                }
                if *filled != Some(Filled::Live) {
                    // Virtual indices meant a loop's frames.
                    pass.forget();
                    *filled = Some(Filled::Live);
                }
                let arrivals = camera.arrivals();
                let Some(newest) = arrivals.last() else {
                    return;
                };
                self.held = paused.then(|| self.held.unwrap_or(newest.time));
                let shown = self.shown(&arrivals);
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

/// Frames played like a clip: a clip's, or a camera loop's.
struct Played {
    /// Which loop the frames are (0 for a clip).
    take: u64,
    count: u32,
    fps: f64,
}

impl Played {
    /// Moves the playhead on, notes it in the trail, and uploads the frames the ring
    /// lacks; `frame` gives frame `i` of the clip or loop.
    #[allow(clippy::too_many_arguments)]
    fn feed<'a>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut FramesPass,
        filled: &mut Option<Filled>,
        playback: &mut Playback,
        trail: &mut Trail,
        v: &VideoFrame,
        time: f64,
        small: bool,
        frame: impl Fn(u32) -> &'a Frame,
    ) -> Sample {
        let fill = Filled::Frames {
            take: self.take,
            mode: v.mode,
        };
        if *filled != Some(fill) {
            // Virtual indices mean other frames in another mode or loop.
            pass.forget();
            *filled = Some(fill);
        }
        let index = playback.index(v, self.count, self.fps);
        trail.record(time, index, f64::from(*ranges::SLIT_DEPTH.end()));
        let sample = clip_sample(v, index, trail, self.count, self.fps, pass.max_layers());
        for k in pass.missing(device, &sample) {
            let frame = frame(clip_frame(v.mode, k, self.count));
            pass.upload(queue, k, if small { &frame.small } else { &frame.full });
        }
        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_takes_memory_from_the_budget_and_gives_it_back() {
        let budget = Budget::new(1000);
        let frame = Arc::new(Frame {
            full: vec![0; 60],
            small: vec![0; 40],
        });
        let frames = vec![frame; 8];
        let looped = Looped::hold(frames.clone(), 30.0, 0.3, 100, &budget).unwrap();
        assert_eq!(looped.count(), 8);
        assert_eq!(budget.available(), 200);
        let err = Looped::hold(frames, 30.0, 0.3, 100, &budget).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("A 0.3 s loop needs"), "{message}");
        assert!(message.contains("is free; shorten the delay"), "{message}");
        drop(looped);
        assert_eq!(budget.available(), 1000);
    }

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
