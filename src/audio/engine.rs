//! The audio engine: one source at a time (none, a live device, or a sound file), read
//! once per canvas frame into [`Signals`], plus hand beats and Pulse.

use std::path::PathBuf;
use std::time::Instant;

use super::analysis::{Analyzer, gained};
use super::file::{Loading, Reanalysis, Sound};
use super::input::{Live, LiveKind};
use super::output::Speakers;
use super::{Beat, Pulse, Shaping, Signal, Signals};
use crate::video::Budget;

/// Where the sound comes from, as the project saves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SourceChoice {
    #[default]
    None,
    /// An input device, by name.
    Input(String),
    /// An output device's loopback, by name.
    Loopback(String),
    /// A sound file.
    File(PathBuf),
}

impl SourceChoice {
    /// Its saved form: `"none"`, `"input:<name>"`, `"loopback:<name>"` or `"file:<path>"`.
    pub fn saved(&self) -> String {
        match self {
            SourceChoice::None => "none".into(),
            SourceChoice::Input(name) => format!("input:{name}"),
            SourceChoice::Loopback(name) => format!("loopback:{name}"),
            SourceChoice::File(path) => format!("file:{}", path.display()),
        }
    }

    /// The choice saved as `saved`, if this version knows the kind.
    pub fn from_saved(saved: &str) -> Option<SourceChoice> {
        if saved == "none" {
            return Some(SourceChoice::None);
        }
        let (kind, rest) = saved.split_once(':')?;
        match kind {
            "input" => Some(SourceChoice::Input(rest.into())),
            "loopback" => Some(SourceChoice::Loopback(rest.into())),
            "file" => Some(SourceChoice::File(PathBuf::from(rest))),
            _ => None,
        }
    }
}

/// Where a file's playhead was at the start of a canvas frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundSpan {
    /// Seconds into the file.
    pub from: f64,
    /// Whether it was playing (a stopped file records silence).
    pub playing: bool,
}

/// One canvas frame of sound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFrame {
    /// The follow signals and beats for this frame.
    pub signals: Signals,
    /// Set when the source is a loaded file: what the soundtrack takes for this frame.
    pub sound: Option<SoundSpan>,
}

/// A loaded file's state, for the panel.
#[derive(Clone, Debug, PartialEq)]
pub struct FileState {
    /// The file's name.
    pub name: String,
    /// Where the playhead is, in seconds.
    pub position: f64,
    /// How long the file is, in seconds.
    pub length: f64,
    /// Whether it is playing.
    pub playing: bool,
}

/// A loaded file and its playhead.
struct FileSource {
    sound: Sound,
    position: f64,
    playing: bool,
    /// Where the playhead was when the last canvas frame was drawn: what the speakers
    /// follow, so the sound matches the picture.
    drawn: f64,
}

/// What is playing now.
enum Current {
    None,
    Live {
        live: Live,
        analyzer: Box<Analyzer>,
        /// Beats found since the last canvas frame.
        beats: [bool; 3],
    },
    File(FileSource),
}

/// A file loading, and the shaping it was analysed with.
struct Pending {
    loading: Loading,
    shaping: Shaping,
}

/// The audio source, its analysis and its playback.
pub struct Audio {
    shaping: Shaping,
    current: Current,
    /// What was asked for (kept while a file loads, or a device is missing).
    choice: SourceChoice,
    loading: Option<Pending>,
    reanalysis: Option<Reanalysis>,
    /// The shaping changed while a re-analysis ran.
    reanalyse_again: bool,
    pulse: Pulse,
    /// Hand beats since the last canvas frame.
    hand: [bool; 3],
    looping: bool,
    volume: f32,
    muted: bool,
    output: String,
    speakers_on: bool,
    speakers: Option<Speakers>,
    /// Why the source or a file load isn't working.
    error: Option<String>,
    /// Why the speakers aren't working (kept apart so reopening them clears only this).
    speaker_error: Option<String>,
    /// Whether the last canvas frame moved time on (it doesn't while the app is paused).
    advancing: bool,
    meter: Signals,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether two shapings analyse a file alike (the gain is applied afterwards).
fn same_analysis(a: Shaping, b: Shaping) -> bool {
    Shaping { gain: b.gain, ..a } == b
}

impl Audio {
    /// No source, no speakers.
    pub fn new() -> Self {
        Self {
            shaping: Shaping::default(),
            current: Current::None,
            choice: SourceChoice::None,
            loading: None,
            reanalysis: None,
            reanalyse_again: false,
            pulse: Pulse::default(),
            hand: [false; 3],
            looping: true,
            volume: 1.0,
            muted: false,
            output: String::new(),
            speakers_on: false,
            speakers: None,
            error: None,
            speaker_error: None,
            advancing: false,
            meter: Signals::default(),
        }
    }

    /// What was asked for.
    pub fn choice(&self) -> &SourceChoice {
        &self.choice
    }

    /// Switches to `choice`. A file loads in the background and the current source keeps
    /// working until it is ready; a device opens at once.
    pub fn use_choice(&mut self, choice: SourceChoice, budget: &Budget) {
        self.choice = choice.clone();
        self.error = None;
        self.loading = None;
        match choice {
            SourceChoice::None => self.stop(),
            SourceChoice::Input(name) => self.open_live(LiveKind::Input, &name),
            SourceChoice::Loopback(name) => self.open_live(LiveKind::Loopback, &name),
            SourceChoice::File(path) => {
                self.loading = Some(Pending {
                    loading: Loading::start(&path, self.shaping, budget),
                    shaping: self.shaping,
                });
            }
        }
    }

    /// Tries the chosen device or file again (Refresh, or after a failure).
    pub fn retry(&mut self, budget: &Budget) {
        let loaded_file = matches!(self.current, Current::File(_))
            && matches!(self.choice, SourceChoice::File(_));
        if !loaded_file {
            self.use_choice(self.choice.clone(), budget);
        }
    }

    fn stop(&mut self) {
        self.current = Current::None;
        self.speakers = None;
        self.speaker_error = None;
        self.reanalysis = None;
        self.reanalyse_again = false;
    }

    fn open_live(&mut self, kind: LiveKind, name: &str) {
        self.stop();
        match Live::open(kind, name) {
            Ok(live) => {
                let analyzer = Box::new(Analyzer::new(live.rate(), self.shaping));
                self.current = Current::Live {
                    live,
                    analyzer,
                    beats: [false; 3],
                };
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    /// Makes `sound` the source, playing from the start.
    pub fn use_sound(&mut self, sound: Sound) {
        self.stop();
        self.current = Current::File(FileSource {
            sound,
            position: 0.0,
            playing: true,
            drawn: 0.0,
        });
        self.open_speakers();
    }

    /// Once per screen refresh: listens to live input, and finishes loads and
    /// re-analyses. Returns the file's name when a load finished, or why it failed.
    pub fn poll(&mut self, now: Instant) -> Option<Result<String, String>> {
        if let Current::Live {
            live,
            analyzer,
            beats,
        } = &mut self.current
        {
            for x in live.drain(now) {
                for (pending, fired) in beats.iter_mut().zip(analyzer.push(x)) {
                    *pending |= fired;
                }
            }
            if let Some(why) = live.failure() {
                self.error = Some(why);
                self.current = Current::None;
            }
        }
        if let Some(why) = self.speakers.as_ref().and_then(Speakers::failure) {
            self.speaker_error = Some(format!("No sound output: {why}"));
            self.speakers = None;
        }
        if let Some(tracks) = self.reanalysis.as_ref().and_then(Reanalysis::poll) {
            self.reanalysis = None;
            if let Current::File(file) = &mut self.current {
                file.sound.tracks = tracks;
                if std::mem::take(&mut self.reanalyse_again) {
                    self.reanalysis =
                        Some(Reanalysis::start(file.sound.samples.clone(), self.shaping));
                }
            }
        }
        let finished = self.loading.as_ref().and_then(|p| p.loading.poll())?;
        let analysed_with = self.loading.take().map(|p| p.shaping)?;
        match finished {
            Ok(sound) => {
                let name = sound.name.clone();
                self.use_sound(sound);
                // The shaping moved while the file loaded: analyse it again.
                if !same_analysis(analysed_with, self.shaping)
                    && let Current::File(file) = &self.current
                {
                    self.reanalysis =
                        Some(Reanalysis::start(file.sound.samples.clone(), self.shaping));
                }
                Some(Ok(name))
            }
            Err(err) => {
                let why = format!("{err:#}");
                self.error = Some(why.clone());
                Some(Err(why))
            }
        }
    }

    /// One canvas frame `dt` seconds long (0 while paused): this frame's signals, and
    /// for a file, where the soundtrack starts.
    pub fn frame(&mut self, dt: f64) -> AudioFrame {
        let mut frame = AudioFrame::default();
        let mut beats = std::mem::take(&mut self.hand);
        let mut values = [0.0; 4];
        let gain = self.shaping.gain;
        match &mut self.current {
            Current::None => {}
            Current::Live {
                analyzer,
                beats: pending,
                ..
            } => {
                values = gained(analyzer.values(), gain);
                for (beat, fired) in beats.iter_mut().zip(std::mem::take(pending)) {
                    *beat |= fired;
                }
            }
            Current::File(file) => {
                let from = file.position;
                file.drawn = from;
                frame.sound = Some(SoundSpan {
                    from,
                    playing: file.playing,
                });
                // A stopped file reads 0; a playing one holds its value while the app
                // is paused (dt is 0), so the frozen picture keeps its modulation.
                if file.playing {
                    values = gained(file.sound.tracks.at(from), gain);
                }
                if file.playing && dt > 0.0 {
                    let length = file.sound.seconds();
                    let mut to = from + dt;
                    let mut ends_here = false;
                    if to >= length {
                        if self.looping && length > 0.0 {
                            to %= length;
                        } else {
                            to = length;
                            ends_here = true;
                            file.playing = false;
                        }
                    }
                    let found = if ends_here {
                        // Stopped at the end: everything up to and including it.
                        file.sound.tracks.beats_in(from, f64::INFINITY)
                    } else {
                        file.sound.tracks.beats_in(from, to)
                    };
                    for (beat, fired) in beats.iter_mut().zip(found) {
                        *beat |= fired;
                    }
                    file.position = to;
                }
            }
        }
        let pulse = self
            .pulse
            .step(beats[Beat::Any.index()], dt as f32, self.shaping.release);
        frame.signals.values[..4].copy_from_slice(&values);
        frame.signals.values[Signal::Pulse.index()] = pulse;
        frame.signals.beats = beats;
        self.meter = frame.signals;
        self.advancing = dt > 0.0;
        self.sync_speakers();
        frame
    }

    /// Stops the speakers where the playhead is, for when canvas frames are about to stop
    /// coming (a dialog, a stall, a minimised window). The next advancing frame resumes
    /// them exactly; without this they would play on and then jump back.
    pub fn hold(&mut self) {
        self.advancing = false;
        self.sync_speakers();
    }

    /// A hand beat, fired on the next canvas frame. A Bass or Treble tap is also Any.
    pub fn tap(&mut self, beat: Beat) {
        self.hand[beat.index()] = true;
        self.hand[Beat::Any.index()] = true;
    }

    /// The shaping in use.
    pub fn shaping(&self) -> Shaping {
        self.shaping
    }

    /// Uses new shaping: at once for live input; a loaded file is re-analysed unless
    /// only the gain changed.
    pub fn set_shaping(&mut self, shaping: Shaping) {
        let shaping = shaping.clamped();
        if shaping == self.shaping {
            return;
        }
        let only_gain = same_analysis(self.shaping, shaping);
        self.shaping = shaping;
        match &mut self.current {
            Current::Live { analyzer, .. } => analyzer.set_shaping(shaping),
            Current::File(file) if !only_gain => {
                if self.reanalysis.is_some() {
                    self.reanalyse_again = true;
                } else {
                    self.reanalysis = Some(Reanalysis::start(file.sound.samples.clone(), shaping));
                }
            }
            _ => {}
        }
    }

    /// Plays or stops a loaded file.
    pub fn set_playing(&mut self, playing: bool) {
        if let Current::File(file) = &mut self.current {
            file.playing = playing && (self.looping || file.position < file.sound.seconds());
        }
    }

    /// Back to the start of a loaded file, playing.
    pub fn restart(&mut self) {
        if let Current::File(file) = &mut self.current {
            file.position = 0.0;
            file.playing = true;
        }
    }

    /// Moves a loaded file's playhead to `seconds`.
    pub fn seek(&mut self, seconds: f64) {
        if let Current::File(file) = &mut self.current {
            file.position = seconds.clamp(0.0, file.sound.seconds());
        }
    }

    /// Sets whether a file wraps to the start at its end.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Whether a file wraps to the start at its end.
    pub fn looping(&self) -> bool {
        self.looping
    }

    /// The loaded file's state, if a file is the source.
    pub fn file(&self) -> Option<FileState> {
        match &self.current {
            Current::File(file) => Some(FileState {
                name: file.sound.name.clone(),
                position: file.position,
                length: file.sound.seconds(),
                playing: file.playing,
            }),
            _ => None,
        }
    }

    /// Whether a non-looping file has played to its end.
    pub fn ended(&self) -> bool {
        match &self.current {
            Current::File(file) => {
                !self.looping && !file.playing && file.position >= file.sound.seconds()
            }
            _ => false,
        }
    }

    /// The soundtrack for a frame: `frames` stereo frames of the file from `span`, or
    /// silence when it wasn't playing (or no file is loaded).
    pub fn soundtrack(&self, span: SoundSpan, frames: usize) -> Vec<i16> {
        match &self.current {
            Current::File(file) if span.playing => {
                file.sound.slice(span.from, frames, self.looping)
            }
            _ => vec![0; frames * 2],
        }
    }

    /// Lets the engine open speakers (the app does; tests don't).
    pub fn enable_speakers(&mut self, on: bool) {
        self.speakers_on = on;
        if on {
            self.open_speakers();
        } else {
            self.speakers = None;
            self.speaker_error = None;
        }
    }

    fn open_speakers(&mut self) {
        self.speakers = None;
        self.speaker_error = None;
        let Current::File(file) = &self.current else {
            return;
        };
        if !self.speakers_on {
            return;
        }
        match Speakers::open(&self.output, file.sound.samples.clone()) {
            Ok(speakers) => self.speakers = Some(speakers),
            Err(err) => self.speaker_error = Some(format!("No sound output: {err:#}")),
        }
        self.sync_speakers();
    }

    /// Whether the speakers should be sounding: the file plays, isn't muted, and time is
    /// moving (a paused app freezes the sound).
    fn speakers_playing(&self) -> bool {
        match &self.current {
            Current::File(file) => file.playing && !self.muted && self.advancing,
            _ => false,
        }
    }

    /// Where the speakers should be: where the last canvas frame was drawn (not where
    /// the playhead moved on to), so the sound doesn't lead the picture by a frame.
    fn speakers_at(&self) -> Option<f64> {
        match &self.current {
            Current::File(file) => Some(file.drawn),
            _ => None,
        }
    }

    fn sync_speakers(&self) {
        if let (Some(speakers), Some(at)) = (&self.speakers, self.speakers_at()) {
            speakers.follow(at, self.speakers_playing(), self.looping, self.volume);
        }
    }

    /// Sets the speakers' volume, 0 to 1.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    /// The speakers' volume, 0 to 1.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Plays on the output device called `name` (`""` for the system default).
    pub fn set_output(&mut self, name: &str) {
        if name != self.output {
            self.output = name.to_string();
            self.open_speakers();
        }
    }

    /// The output device's name (`""` for the system default).
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Silences the speakers (during a frame-by-frame recording) without stopping the
    /// file.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The file loading and how far it has got, if one is.
    pub fn loading(&self) -> Option<(String, f32)> {
        self.loading.as_ref().map(|p| {
            let path = &p.loading.path;
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            (name, p.loading.progress())
        })
    }

    /// Why the source or the speakers aren't working, if they aren't (the source's
    /// reason first).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref().or(self.speaker_error.as_deref())
    }

    /// The last frame's signals, for the meter.
    pub fn meter(&self) -> Signals {
        self.meter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::RATE;
    use crate::audio::file::Sound;
    use crate::video::{Budget, MEMORY_LIMIT};
    use std::f32::consts::TAU;

    const FRAME: f64 = 1.0 / 60.0;

    /// Interleaved stereo: a full-scale 1 kHz tone for `tone` seconds, then silence for
    /// `rest`.
    fn tone_then_rest(tone: f64, rest: f64) -> Vec<i16> {
        let on = (tone * RATE as f64) as usize;
        let off = (rest * RATE as f64) as usize;
        (0..on + off)
            .flat_map(|i| {
                let v = if i < on {
                    ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 32767.0) as i16
                } else {
                    0
                };
                [v, v]
            })
            .collect()
    }

    /// `count` 20 ms clicks, `gap` seconds apart.
    fn clicks(count: usize, gap: f64) -> Vec<i16> {
        let every = (gap * RATE as f64) as usize;
        let burst = (0.02 * RATE as f64) as usize;
        (0..count * every)
            .flat_map(|i| {
                let v = if i % every < burst {
                    ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 30000.0) as i16
                } else {
                    0
                };
                [v, v]
            })
            .collect()
    }

    fn with_sound(samples: Vec<i16>) -> Audio {
        let budget = Budget::new(MEMORY_LIMIT);
        let sound = Sound::new("test".into(), samples, Shaping::default(), &budget).unwrap();
        let mut audio = Audio::new();
        audio.use_sound(sound);
        audio
    }

    fn run(audio: &mut Audio, seconds: f64) -> (Signals, usize) {
        let mut beats = 0;
        let mut last = Signals::default();
        let mut t = 0.0;
        while t < seconds - 1e-9 {
            last = audio.frame(FRAME).signals;
            beats += usize::from(last.beat(Beat::Any));
            t += FRAME;
        }
        (last, beats)
    }

    #[test]
    fn source_names_round_trip() {
        for choice in [
            SourceChoice::None,
            SourceChoice::Input("Microphone (USB)".into()),
            SourceChoice::Loopback("Speakers: 2".into()),
            SourceChoice::File(PathBuf::from("C:/music/drums.wav")),
        ] {
            assert_eq!(
                SourceChoice::from_saved(&choice.saved()),
                Some(choice.clone())
            );
        }
        assert_eq!(SourceChoice::None.saved(), "none");
        assert_eq!(SourceChoice::from_saved("radio:bbc"), None);
    }

    #[test]
    fn no_source_reads_zero_but_hand_beats_pulse() {
        let mut audio = Audio::new();
        assert_eq!(audio.frame(FRAME).signals, Signals::default());
        audio.tap(Beat::Bass);
        let s = audio.frame(FRAME).signals;
        assert!(s.beat(Beat::Bass) && s.beat(Beat::Any) && !s.beat(Beat::Treble));
        assert_eq!(s.get(Signal::Pulse), 1.0);
        assert_eq!(s.get(Signal::Level), 0.0);
        let next = audio.frame(FRAME).signals;
        assert!(!next.beat(Beat::Any), "a tap fires once");
        assert!(next.get(Signal::Pulse) < 1.0, "and Pulse falls");
    }

    #[test]
    fn a_file_reads_its_tracks_at_the_playhead() {
        let mut audio = with_sound(tone_then_rest(1.0, 1.0));
        let (during, _) = run(&mut audio, 0.5);
        assert!(during.get(Signal::Level) > 0.8, "{during:?}");
        let (after, _) = run(&mut audio, 1.0);
        assert!(after.get(Signal::Level) < 0.1, "{after:?}");
        let state = audio.file().unwrap();
        assert!((state.position - 1.5).abs() < 2.0 * FRAME);
        assert_eq!(state.length, 2.0);
    }

    #[test]
    fn each_beat_fires_once_and_again_on_the_next_loop() {
        let mut audio = with_sound(clicks(4, 0.5));
        audio.set_looping(true);
        let (_, beats) = run(&mut audio, 2.0);
        assert_eq!(beats, 4);
        let (_, beats) = run(&mut audio, 2.0);
        assert_eq!(beats, 4, "the second time round");
    }

    #[test]
    fn without_loop_the_file_stops_at_its_end() {
        let mut audio = with_sound(clicks(2, 0.5));
        audio.set_looping(false);
        run(&mut audio, 1.5);
        let state = audio.file().unwrap();
        assert!(!state.playing);
        assert_eq!(state.position, 1.0);
        assert!(audio.ended());
        audio.restart();
        assert!(!audio.ended());
        assert_eq!(audio.file().unwrap().position, 0.0);
    }

    #[test]
    fn a_paused_frame_neither_moves_nor_fires() {
        let mut audio = with_sound(clicks(4, 0.5));
        let frame = audio.frame(0.0);
        assert_eq!(frame.signals.beats, [false; 3]);
        assert_eq!(audio.file().unwrap().position, 0.0);
        assert_eq!(
            frame.sound,
            Some(SoundSpan {
                from: 0.0,
                playing: true
            })
        );
    }

    #[test]
    fn the_soundtrack_comes_from_the_file_at_the_span() {
        let ramp: Vec<i16> = (0..100).flat_map(|i| [i as i16, i as i16]).collect();
        let mut audio = with_sound(ramp);
        audio.set_looping(false);
        let span = SoundSpan {
            from: 2.0 / RATE as f64,
            playing: true,
        };
        assert_eq!(audio.soundtrack(span, 2), [2, 2, 3, 3]);
        let stopped = SoundSpan {
            playing: false,
            ..span
        };
        assert_eq!(audio.soundtrack(stopped, 2), [0; 4]);
        assert_eq!(Audio::new().soundtrack(span, 2), [0; 4], "no file, silence");
    }

    #[test]
    fn a_paused_app_asks_the_speakers_to_stop() {
        let mut audio = with_sound(tone_then_rest(1.0, 1.0));
        audio.frame(FRAME);
        assert!(audio.speakers_playing(), "a moving frame plays");
        audio.frame(0.0);
        assert!(!audio.speakers_playing(), "a paused frame stops");
        audio.frame(FRAME);
        assert!(audio.speakers_playing());
        audio.set_muted(true);
        audio.frame(FRAME);
        assert!(!audio.speakers_playing(), "muted stops");
    }

    #[test]
    fn holding_stops_the_speakers_until_the_next_moving_frame() {
        let mut audio = with_sound(tone_then_rest(1.0, 1.0));
        audio.frame(FRAME);
        assert!(audio.speakers_playing());
        audio.hold();
        assert!(!audio.speakers_playing(), "held");
        audio.frame(FRAME);
        assert!(audio.speakers_playing(), "the next moving frame resumes");
    }

    #[test]
    fn reopening_the_speakers_clears_only_their_error() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        audio.speaker_error = Some("No sound output: gone".into());
        assert_eq!(audio.error(), Some("No sound output: gone"));
        audio.error = Some("load failed".into());
        assert_eq!(
            audio.error(),
            Some("load failed"),
            "the source's reason first"
        );
        // Speakers stay disabled here, so no stream opens; the reopen path still runs.
        audio.set_output("other");
        assert_eq!(audio.speaker_error, None);
        assert_eq!(audio.error(), Some("load failed"), "the source error stays");
    }

    #[test]
    fn gain_applies_without_reanalysis() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        let (full, _) = run(&mut audio, 0.5);
        audio.set_shaping(Shaping {
            gain: 0.5,
            ..Shaping::default()
        });
        assert!(audio.reanalysis.is_none(), "gain needs no re-run");
        let half = audio.frame(FRAME).signals;
        assert!((half.get(Signal::Level) - full.get(Signal::Level) / 2.0).abs() < 0.05);
    }

    #[test]
    fn an_ended_file_reads_zero() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        audio.set_looping(false);
        let (during, _) = run(&mut audio, 0.5);
        assert!(during.get(Signal::Level) > 0.8, "{during:?}");
        run(&mut audio, 1.0);
        assert!(audio.ended());
        let after = audio.frame(FRAME).signals;
        assert_eq!(after.values[..4], [0.0; 4], "{after:?}");
    }

    #[test]
    fn a_file_paused_with_its_own_button_reads_zero() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        run(&mut audio, 0.5);
        audio.set_playing(false);
        let paused = audio.frame(FRAME).signals;
        assert_eq!(paused.values[..4], [0.0; 4], "{paused:?}");
    }

    #[test]
    fn a_paused_app_holds_a_playing_file_s_value() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        let (playing, _) = run(&mut audio, 0.5);
        let held = audio.frame(0.0).signals;
        assert!(held.get(Signal::Level) > 0.8, "{held:?}");
        assert!((held.get(Signal::Level) - playing.get(Signal::Level)).abs() < 0.05);
    }

    #[test]
    fn the_speakers_follow_where_the_frame_was_drawn() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        let frame = audio.frame(FRAME);
        assert_eq!(frame.sound.unwrap().from, 0.0);
        assert_eq!(
            audio.file().unwrap().position,
            FRAME,
            "the playhead moved on"
        );
        assert_eq!(
            audio.speakers_at(),
            Some(0.0),
            "the speakers stay with the picture"
        );
        audio.frame(FRAME);
        assert_eq!(audio.speakers_at(), Some(FRAME));
        assert_eq!(Audio::new().speakers_at(), None);
    }

    #[test]
    fn a_new_release_reanalyses_the_file() {
        let mut audio = with_sound(tone_then_rest(0.5, 1.0));
        audio.set_shaping(Shaping {
            release: 1.0,
            ..Shaping::default()
        });
        let start = Instant::now();
        while audio.reanalysis.is_some() && start.elapsed().as_secs() < 10 {
            audio.poll(Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        audio.seek(0.8);
        let s = audio.frame(FRAME).signals;
        assert!(
            s.get(Signal::Level) > 0.5,
            "0.3 s after the tone a 1 s release still holds: {s:?}"
        );
    }
}
