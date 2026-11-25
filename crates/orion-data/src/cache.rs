//! Element Sets kept on disk between runs.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use orion_core::catalogue::CatalogueRecords;

use crate::{ElementSetFiles, Loaded, parse_catalogue};

const SATELLITES: &str = "satellites.csv";
const ROCKET_BODIES: &str = "rocket_bodies.csv";
const DEBRIS: &str = "debris.csv";
/// Holds the time the files were fetched, as seconds since 1970.
const FETCHED_AT: &str = "fetched_at";
/// Holds the time of the last attempt to fetch, successful or not, as seconds since 1970.
const LAST_ATTEMPT: &str = "last_attempt";
/// CelesTrak's satellite catalogue, as fetched.
const CATALOGUE: &str = "catalogue.csv";
/// Holds the time the catalogue was fetched, as seconds since 1970.
const CATALOGUE_FETCHED_AT: &str = "catalogue_fetched_at";

/// A folder holding the most recently fetched Element Sets.
#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// The cache in the operating system's standard place for one, or `None` if the
    /// system does not say where that is.
    pub fn in_default_location() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("", "", "Orion")?;
        Some(Self::at(dirs.cache_dir()))
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The stored Element Sets, or `None` if there are none or they cannot be read.
    pub fn load(&self) -> Option<Loaded> {
        let fetched_at = self.read_time(FETCHED_AT)?;
        let read = |name| fs::read_to_string(self.dir.join(name)).ok();
        let files = ElementSetFiles {
            satellites: read(SATELLITES)?,
            rocket_bodies: read(ROCKET_BODIES)?,
            debris: read(DEBRIS)?,
        };
        Loaded::parse(&files, fetched_at)
    }

    /// Replaces the stored Element Sets.
    pub fn store(&self, files: &ElementSetFiles, fetched_at: SystemTime) -> io::Result<()> {
        self.write(SATELLITES, &files.satellites)?;
        self.write(ROCKET_BODIES, &files.rocket_bodies)?;
        self.write(DEBRIS, &files.debris)?;
        // Written last: its presence marks the set as complete.
        self.write_time(FETCHED_AT, fetched_at)
    }

    /// The stored Catalogue Records and when they were fetched, or `None` if there are
    /// none or they cannot be read.
    pub fn load_catalogue(&self) -> Option<(CatalogueRecords, SystemTime)> {
        let fetched_at = self.catalogue_fetched_at()?;
        let text = fs::read_to_string(self.dir.join(CATALOGUE)).ok()?;
        Some((parse_catalogue(&text)?, fetched_at))
    }

    /// Replaces the stored catalogue file.
    pub fn store_catalogue(&self, text: &str, fetched_at: SystemTime) -> io::Result<()> {
        self.write(CATALOGUE, text)?;
        self.write_time(CATALOGUE_FETCHED_AT, fetched_at)
    }

    pub fn catalogue_fetched_at(&self) -> Option<SystemTime> {
        self.read_time(CATALOGUE_FETCHED_AT)
    }

    /// When a fetch was last attempted, whether or not it succeeded.
    pub fn last_attempt(&self) -> Option<SystemTime> {
        self.read_time(LAST_ATTEMPT)
    }

    pub fn record_attempt(&self, at: SystemTime) -> io::Result<()> {
        self.write_time(LAST_ATTEMPT, at)
    }

    /// Writes a file so that a reader sees either the old contents or the new, never a mix.
    fn write(&self, name: &str, contents: &str) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let partial = self.dir.join(format!("{name}.partial"));
        fs::write(&partial, contents)?;
        fs::rename(partial, self.dir.join(name))
    }

    fn write_time(&self, name: &str, time: SystemTime) -> io::Result<()> {
        let seconds = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        self.write(name, &seconds.to_string())
    }

    fn read_time(&self, name: &str) -> Option<SystemTime> {
        let seconds: u64 = fs::read_to_string(self.dir.join(name))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{catalogue, files, hours};

    #[test]
    fn stored_catalogue_records_load_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        assert!(cache.load_catalogue().is_none());
        assert_eq!(cache.catalogue_fetched_at(), None);

        let fetched_at = SystemTime::UNIX_EPOCH + hours(7);
        cache.store_catalogue(&catalogue(), fetched_at).unwrap();
        let (records, loaded_at) = cache.load_catalogue().expect("stored Catalogue Records");
        assert_eq!(records[&25_544].launch_date, "1998-11-20");
        assert_eq!(loaded_at, fetched_at);

        // A damaged file is ignored, not half-read.
        fs::write(dir.path().join(CATALOGUE), "garbage").unwrap();
        assert!(cache.load_catalogue().is_none());
    }

    #[test]
    fn an_empty_cache_has_nothing_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("not-created-yet"));
        assert!(cache.load().is_none());
        assert_eq!(cache.last_attempt(), None);
    }

    #[test]
    fn stored_element_sets_load_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path().join("nested").join("cache"));
        let fetched_at = SystemTime::UNIX_EPOCH + hours(500_000);

        cache
            .store(&files(), fetched_at)
            .expect("a writable folder");
        let loaded = cache.load().expect("stored Element Sets");

        assert_eq!(loaded.fetched_at, fetched_at);
        assert_eq!(loaded.objects.len(), 3);
        // No half-written files are left behind.
        let leftovers = fs::read_dir(cache.dir())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".partial")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn a_damaged_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        cache
            .store(&files(), SystemTime::UNIX_EPOCH + hours(1))
            .unwrap();

        fs::write(dir.path().join(DEBRIS), "this is not a CSV of Element Sets").unwrap();
        assert!(cache.load().is_none());
    }

    #[test]
    fn an_incomplete_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        cache
            .store(&files(), SystemTime::UNIX_EPOCH + hours(1))
            .unwrap();

        fs::remove_file(dir.path().join(FETCHED_AT)).unwrap();
        assert!(cache.load().is_none());
    }

    #[test]
    fn attempts_are_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let at = SystemTime::UNIX_EPOCH + hours(42);

        cache.record_attempt(at).unwrap();
        assert_eq!(cache.last_attempt(), Some(at));
    }
}
