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

/// An open input port.
struct Open {
    id: String,
    name: String,
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

    /// Opens ports that appeared, drops ones that went, and tries again ones that
    /// failed. Ports already open stay as they are.
    pub fn rescan(&mut self, now: Instant) {
        self.scanned = Some(now);
        self.failed.clear();
        let ports = match list() {
            Ok(ports) => ports,
            Err(err) => {
                self.open.clear();
                self.failed.push(("MIDI".into(), err));
                return;
            }
        };
        self.open
            .retain(|o| ports.iter().any(|(id, _)| *id == o.id));
        for (id, name) in ports {
            if self.open.iter().any(|o| o.id == id) {
                continue;
            }
            match connect(&id, self.sender.clone()) {
                Ok(connection) => self.open.push(Open {
                    id,
                    name,
                    _connection: connection,
                }),
                Err(err) => {
                    log::warn!("could not open MIDI input {name}: {err}");
                    self.failed.push((name, err));
                }
            }
        }
    }

    /// The open devices' names.
    pub fn devices(&self) -> Vec<String> {
        self.open.iter().map(|o| o.name.clone()).collect()
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

/// The MIDI input ports there are now, as (id, name).
pub fn list() -> Result<Vec<(String, String)>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    Ok(input
        .ports()
        .iter()
        .map(|port| {
            let name = input
                .port_name(port)
                .unwrap_or_else(|_| "MIDI device".into());
            (port.id(), name)
        })
        .collect())
}

/// Opens the port with id `id`, sending each message a link can use to `sender`.
fn connect(id: &str, sender: Sender<Message>) -> Result<midir::MidiInputConnection<()>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    let port = input
        .find_port_by_id(id)
        .ok_or_else(|| "it was unplugged".to_string())?;
    input
        .connect(
            &port,
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
