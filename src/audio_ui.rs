//! The panel's Audio section: the sound source, its settings, a meter, hand-beat
//! buttons and the audio links. It only reports what was asked for; the app does it.

/// What the Audio section shows and edits, kept between frames.
#[derive(Clone, Debug)]
pub struct AudioUi {
    /// Whether Record restarts the sound file from its start.
    pub start_with_recording: bool,
}

impl Default for AudioUi {
    fn default() -> Self {
        Self {
            start_with_recording: true,
        }
    }
}
