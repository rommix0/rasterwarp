//! The panel's light or dark appearance, and the few colors the panel picks itself, so
//! they read on either.

use egui::{Color32, ThemePreference, Ui};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which appearance the panel uses (saved in the settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Follow Windows' light or dark app mode.
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    /// The name the settings file uses.
    pub fn name(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
        }
    }

    /// What the panel shows.
    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
        }
    }

    /// The choice saved as `name`, if it is one.
    pub fn from_name(name: &str) -> Option<ThemeChoice> {
        ThemeChoice::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The preference egui follows.
    pub fn preference(self) -> ThemePreference {
        match self {
            ThemeChoice::System => ThemePreference::System,
            ThemeChoice::Dark => ThemePreference::Dark,
            ThemeChoice::Light => ThemePreference::Light,
        }
    }
}

impl Serialize for ThemeChoice {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// An unknown name reads as System, so a bad value never costs the other settings.
impl<'de> Deserialize<'de> for ThemeChoice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        Ok(value
            .as_str()
            .and_then(ThemeChoice::from_name)
            .unwrap_or_else(|| {
                log::warn!("ignoring an unknown theme: {value}");
                ThemeChoice::default()
            }))
    }
}

/// Error lines: light red on dark, a deeper red on light.
pub fn error(ui: &Ui) -> Color32 {
    pick(ui, Color32::LIGHT_RED, Color32::from_rgb(190, 20, 20))
}

/// Notes and warnings that aren't errors: yellow on dark, amber on light.
pub fn note(ui: &Ui) -> Color32 {
    pick(ui, Color32::YELLOW, Color32::from_rgb(160, 95, 0))
}

/// The curve editor's curve: orange, deeper on light.
pub fn curve(ui: &Ui) -> Color32 {
    pick(
        ui,
        Color32::from_rgb(255, 170, 60),
        Color32::from_rgb(210, 105, 0),
    )
}

fn pick(ui: &Ui, dark: Color32, light: Color32) -> Color32 {
    if ui.visuals().dark_mode { dark } else { light }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_reads_as_system() {
        for t in ThemeChoice::ALL {
            let text = serde_json::to_string(&t).unwrap();
            assert_eq!(text, format!("\"{}\"", t.name()));
            assert_eq!(serde_json::from_str::<ThemeChoice>(&text).unwrap(), t);
        }
        assert_eq!(
            serde_json::from_str::<ThemeChoice>("\"purple\"").unwrap(),
            ThemeChoice::System
        );
        assert_eq!(
            serde_json::from_str::<ThemeChoice>("3").unwrap(),
            ThemeChoice::System
        );
    }
}
