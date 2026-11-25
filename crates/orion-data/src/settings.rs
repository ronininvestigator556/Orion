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
}

impl Settings {
    /// Whether a switch was on, or `None` if nothing is remembered about it.
    pub fn get(&self, name: &str) -> Option<bool> {
        self.switches.get(name).copied()
    }

    pub fn set(&mut self, name: &str, on: bool) {
        self.switches.insert(name.to_owned(), on);
    }

    /// Reads settings written by [`Self::to_text`]. Lines that cannot be read are
    /// passed over, so a file from a newer or older version does no harm.
    pub fn parse(text: &str) -> Self {
        let switches = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .filter_map(|line| {
                let (name, value) = line.rsplit_once('=')?;
                let on = match value.trim() {
                    "on" => true,
                    "off" => false,
                    _ => return None,
                };
                Some((name.trim().to_owned(), on))
            })
            .filter(|(name, _)| !name.is_empty())
            .collect();
        Self { switches }
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
