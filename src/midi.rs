//! MIDI input: every input port is open at once, and each message goes to the app over
//! a channel, so neither rendering nor the panel ever waits on a controller. Ports are
//! looked for again every few seconds, since Windows doesn't say when a device is
//! plugged in.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::control::Message;

/// How often the ports are looked for again.
pub const RESCAN: Duration = Duration::from_secs(3);

/// The client name midir gives the system.
const CLIENT: &str = "rasterwarp";

/// A port as it's told apart: its id and its name. An id alone isn't enough, since on
/// Windows the ports of one multi-port device can share it.
type Key = (String, String);

/// An open input port.
struct Open {
    key: Key,
    /// Closes the port when dropped.
    _connection: midir::MidiInputConnection<()>,
}

/// The open MIDI inputs and the messages they've sent.
pub struct Midi {
    open: Vec<Open>,
    /// Ports that wouldn't open, and why; tried again at the next rescan.
    failed: Vec<(String, String)>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    /// When the ports were last looked for.
    scanned: Option<Instant>,
}

impl Default for Midi {
    fn default() -> Self {
        Midi::new()
    }
}

impl Midi {
    /// No ports yet: the first [`Midi::poll`] looks for them.
    pub fn new() -> Midi {
        let (sender, receiver) = mpsc::channel();
        Midi {
            open: Vec::new(),
            failed: Vec::new(),
            sender,
            receiver,
            scanned: None,
        }
    }

    /// The messages that arrived since the last call, oldest first, after looking for
    /// ports again if [`RESCAN`] has passed.
    pub fn poll(&mut self, now: Instant) -> Vec<Message> {
        if self.rescan_due(now) {
            self.rescan(now);
        }
        self.drain()
    }

    /// Closes every port and opens them all again: the Refresh button. This is also the
    /// way back for a device that was unplugged and replugged between two rescans, whose
    /// old connection looks open but is dead.
    pub fn refresh(&mut self, now: Instant) {
        self.open.clear();
        self.rescan(now);
    }

    /// Opens ports that appeared, drops ones that went, and tries again ones that
    /// failed. Ports already open stay as they are.
    fn rescan(&mut self, now: Instant) {
        self.scanned = Some(now);
        let tried = std::mem::take(&mut self.failed);
        let found = match scan() {
            Ok(found) => found,
            Err(err) => {
                self.open.clear();
                self.failed.push(("MIDI".into(), err));
                return;
            }
        };
        let keys: Vec<Key> = found
            .iter()
            .map(|(port, name)| (port.id(), name.clone()))
            .collect();
        self.open.retain(|o| keys.contains(&o.key));
        let open: Vec<Key> = self.open.iter().map(|o| o.key.clone()).collect();
        for i in unopened(&open, &keys) {
            let (port, name) = &found[i];
            match connect(port, self.sender.clone()) {
                Ok(connection) => self.open.push(Open {
                    key: keys[i].clone(),
                    _connection: connection,
                }),
                Err(err) => {
                    // A port another app holds fails every time; say so once.
                    let failure = (name.clone(), err);
                    if !tried.contains(&failure) {
                        log::warn!("could not open MIDI input {}: {}", failure.0, failure.1);
                    }
                    self.failed.push(failure);
                }
            }
        }
    }

    /// The open devices' names.
    pub fn devices(&self) -> Vec<String> {
        self.open.iter().map(|o| o.key.1.clone()).collect()
    }

    /// Ports that wouldn't open at the last rescan, and why.
    pub fn failed(&self) -> &[(String, String)] {
        &self.failed
    }

    fn rescan_due(&self, now: Instant) -> bool {
        self.scanned
            .is_none_or(|then| now.saturating_duration_since(then) >= RESCAN)
    }

    fn drain(&mut self) -> Vec<Message> {
        self.receiver.try_iter().collect()
    }
}

/// The indices of `found` that aren't in `open`, in order.
fn unopened(open: &[Key], found: &[Key]) -> Vec<usize> {
    (0..found.len())
        .filter(|&i| !open.contains(&found[i]) && !found[..i].contains(&found[i]))
        .collect()
}

/// The MIDI input ports there are now, with their names.
fn scan() -> Result<Vec<(midir::MidiInputPort, String)>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    Ok(input
        .ports()
        .into_iter()
        .map(|port| {
            let name = input
                .port_name(&port)
                .unwrap_or_else(|_| "MIDI device".into());
            (port, name)
        })
        .collect())
}

/// The MIDI input ports there are now, as (id, name).
pub fn list() -> Result<Vec<(String, String)>, String> {
    Ok(scan()?
        .into_iter()
        .map(|(port, name)| (port.id(), name))
        .collect())
}

/// Opens `port`, sending each message a link can use to `sender`.
fn connect(
    port: &midir::MidiInputPort,
    sender: Sender<Message>,
) -> Result<midir::MidiInputConnection<()>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    input
        .connect(
            port,
            "rasterwarp-in",
            move |_, bytes, _| {
                if let Some(message) = Message::parse(bytes) {
                    let _ = sender.send(message);
                }
            },
            (),
        )
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Message;

    #[test]
    fn listing_ports_works_with_or_without_devices() {
        let ports = list().expect("listing MIDI ports");
        for (id, name) in &ports {
            assert!(!id.is_empty(), "{name} has no id");
        }
    }

    #[test]
    fn ports_sharing_an_id_are_told_apart_by_name() {
        let key = |id: &str, name: &str| (id.to_string(), name.to_string());
        let found = [
            key("usb1", "Pad 1"),
            key("usb1", "Pad 2"),
            key("usb2", "Keys"),
        ];
        assert_eq!(
            unopened(&[], &found),
            [0, 1, 2],
            "the second port isn't skipped"
        );
        assert_eq!(unopened(&[found[0].clone()], &found), [1, 2]);
        assert_eq!(unopened(&found, &found), Vec::<usize>::new());
    }

    #[test]
    fn messages_wait_until_drained_in_order() {
        let mut midi = Midi::new();
        let first = Message::Cc {
            channel: 1,
            number: 1,
            value: 3,
        };
        let second = Message::NoteOn {
            channel: 1,
            number: 60,
        };
        midi.sender.send(first).unwrap();
        midi.sender.send(second).unwrap();
        assert_eq!(midi.drain(), [first, second]);
        assert!(midi.drain().is_empty());
        assert!(
            midi.devices().is_empty() && midi.failed().is_empty(),
            "nothing opened"
        );
    }

    #[test]
    fn a_rescan_is_due_at_first_and_then_every_few_seconds() {
        let mut midi = Midi::new();
        let start = Instant::now();
        assert!(midi.rescan_due(start));
        midi.scanned = Some(start);
        assert!(!midi.rescan_due(start + RESCAN / 2));
        assert!(midi.rescan_due(start + RESCAN));
    }
}
