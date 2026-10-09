//! The panel's MIDI section: the devices, the links, and learning a new one.

use crate::control::Links;

/// What the MIDI section shows and edits.
#[derive(Clone, Debug, Default)]
pub struct MidiUi {
    /// The links in use; the app applies them and saves them with the settings.
    pub links: Links,
    /// The open devices' names.
    pub devices: Vec<String>,
    /// Ports that wouldn't open, and why.
    pub failed: Vec<(String, String)>,
}
