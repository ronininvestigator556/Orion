//! What the user arranged last time, kept between runs: which Layers were on and
//! what the Base Map showed. Nothing else about the user is kept.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Switches by name, each on or off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    switches: BTreeMap<String, bool>,
    /// Settings that are a choice or an address, not a switch.
    texts: BTreeMap<String, String>,
}

impl Settings {
    /// Whether a switch was on, or `None` if nothing is remembered about it.
    pub fn get(&self, name: &str) -> Option<bool> {
        self.switches.get(name).copied()
    }

    pub fn set(&mut self, name: &str, on: bool) {
        self.switches.insert(name.to_owned(), on);
    }

    /// What a setting that is not a switch was set to, or `None` if nothing is
    /// remembered about it.
    pub fn get_text(&self, name: &str) -> Option<&str> {
        self.texts.get(name).map(String::as_str)
    }

    pub fn set_text(&mut self, name: &str, value: &str) {
        self.texts.insert(name.to_owned(), value.trim().to_owned());
    }

    /// Reads settings written by [`Self::to_text`]. Lines that cannot be read are
    /// passed over, so a file from a newer or older version does no harm.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            // The name ends at the first equals sign: an address may hold more.
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() || value.is_empty() {
                continue;
            }
            match value {
                "on" => settings.set(name, true),
                "off" => settings.set(name, false),
                other => settings.set_text(name, other),
            }
        }
        settings
    }

    /// Writes the settings as text a person can read: a line for each switch.
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# Orion's settings: what was switched on when it was last closed.\n\
             # Written by the app. Deleting this file puts everything back as it first was.\n",
        );
        for (name, on) in &self.switches {
            text.push_str(name);
            text.push_str(if *on { " = on\n" } else { " = off\n" });
        }
        for (name, value) in &self.texts {
            text.push_str(&format!("{name} = {value}\n"));
        }
        text
    }
}

/// The file settings are kept in.
#[derive(Clone, Debug)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    /// The file in the operating system's standard place for an app's settings, or
    /// `None` if the system does not say where that is.
    pub fn in_default_location() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "Orion")?;
        Some(Self::at(dirs.config_dir().join("settings.txt")))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings kept from last time; none at all if there is no file yet.
    pub fn load(&self) -> Settings {
        fs::read_to_string(&self.path)
            .map(|text| Settings::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(&self, settings: &Settings) -> io::Result<()> {
        if let Some(folder) = self.path.parent() {
            fs::create_dir_all(folder)?;
        }
        // Written beside the final name and renamed, so a crash leaves the old file whole.
        let partial = self.path.with_extension("partial");
        fs::write(&partial, settings.to_text())?;
        fs::rename(partial, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_being_written_and_read() {
        let mut settings = Settings::default();
        settings.set("layer.Earthquakes", true);
        settings.set("layer.U.S. weather alerts", false);
        settings.set("base map.borders", true);
        assert_eq!(Settings::parse(&settings.to_text()), settings);
        assert_eq!(settings.get("layer.Earthquakes"), Some(true));
        assert_eq!(settings.get("layer.U.S. weather alerts"), Some(false));
        assert_eq!(settings.get("layer.Flights"), None);
    }

    #[test]
    fn a_choice_or_an_address_is_kept_as_it_was_written() {
        let mut settings = Settings::default();
        settings.set("base map.borders", true);
        settings.set_text("base map.street map", "automatic");
        settings.set_text(
            "street map.address",
            "https://maps.example/tiles?key=abc&z={z}&x={x}&y={y}",
        );
        let read = Settings::parse(&settings.to_text());
        assert_eq!(read, settings);
        assert_eq!(read.get_text("base map.street map"), Some("automatic"));
        // An address may have equals signs of its own.
        assert_eq!(
            read.get_text("street map.address"),
            Some("https://maps.example/tiles?key=abc&z={z}&x={x}&y={y}")
        );
        assert_eq!(read.get_text("base map.borders"), None);
        assert_eq!(read.get("base map.street map"), None);
        assert_eq!(read.get_text("never set"), None);
    }

    #[test]
    fn lines_that_make_no_sense_are_passed_over() {
        let settings = Settings::parse(
            "# a comment = on\nlayer.A = on\nlayer.B = maybe\nno equals sign\n = on\nlayer.C=off\n",
        );
        assert_eq!(settings.get("layer.A"), Some(true));
        assert_eq!(settings.get("layer.B"), None);
        assert_eq!(settings.get("layer.C"), Some(false));
        assert_eq!(settings.get("# a comment"), None);
    }

    #[test]
    fn a_file_keeps_settings_between_runs() {
        let folder = tempfile::tempdir().unwrap();
        let file = SettingsFile::at(folder.path().join("deeper").join("settings.txt"));
        assert_eq!(file.load(), Settings::default());

        let mut settings = Settings::default();
        settings.set("layer.Flights", true);
        file.save(&settings).unwrap();
        assert_eq!(file.load(), settings);
        settings.set("layer.Flights", false);
        file.save(&settings).unwrap();
        assert_eq!(file.load().get("layer.Flights"), Some(false));
    }
}
