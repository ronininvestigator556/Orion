//! Fetching and caching Element Sets for Orion.
//!
//! Element Sets come from a [`Source`] on the network and are kept in a [`Cache`] on
//! disk, so the app can start without a connection. [`refresh`] ties the two together
//! and enforces how often a Source may be asked.

mod cache;
mod source;
#[cfg(test)]
mod test_support;

use std::fmt;
use std::io;
use std::time::{Duration, SystemTime};

use orion_core::catalogue::{ObjectKind, TrackedObject, parse_element_set_files};

pub use cache::Cache;
pub use source::{CelesTrak, FetchError, Source};

/// Element Sets older than this are refreshed automatically.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Minimum time between two attempts to fetch, whether or not the first succeeded.
///
/// CelesTrak updates its data every two hours and blocks addresses that ask more often
/// or that keep asking after an error, so this is deliberately strict.
pub const ATTEMPT_LOCK: Duration = Duration::from_secs(2 * 60 * 60);

/// Raw Element Set files, one per kind of Tracked Object, as a Source provides them.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementSetFiles {
    pub satellites: String,
    pub rocket_bodies: String,
    pub debris: String,
}

impl ElementSetFiles {
    /// The files with their kinds, most trusted kind first.
    pub fn by_kind(&self) -> [(ObjectKind, &str); 3] {
        [
            (ObjectKind::Satellite, &self.satellites),
            (ObjectKind::RocketBody, &self.rocket_bodies),
            (ObjectKind::Debris, &self.debris),
        ]
    }
}

/// Tracked Objects read from fetched or cached Element Sets.
#[derive(Debug)]
pub struct Loaded {
    pub objects: Vec<TrackedObject>,
    /// Element Sets describing orbits the model cannot represent.
    pub rejected: usize,
    /// When the Element Sets were fetched from their Source.
    pub fetched_at: SystemTime,
}

/// First column of every CSV Element Set file.
const CSV_HEADER_START: &str = "OBJECT_NAME,";

/// True if the text begins like a CSV Element Set file. Without this check, arbitrary
/// text would pass as a file with a strange header and no rows.
fn has_element_set_header(text: &str) -> bool {
    text.starts_with(CSV_HEADER_START)
}

impl Loaded {
    fn parse(files: &ElementSetFiles, fetched_at: SystemTime) -> Option<Self> {
        if !files
            .by_kind()
            .iter()
            .all(|(_, text)| has_element_set_header(text))
        {
            return None;
        }
        let parsed = parse_element_set_files(&files.by_kind()).ok()?;
        (!parsed.objects.is_empty()).then_some(Self {
            objects: parsed.objects,
            rejected: parsed.rejected,
            fetched_at,
        })
    }
}

/// True if Element Sets obtained at `obtained_at` are due an automatic refresh.
pub fn is_stale(obtained_at: SystemTime, now: SystemTime) -> bool {
    // A time in the future means the clock was changed; treat the data as fresh.
    now.duration_since(obtained_at)
        .is_ok_and(|age| age > STALE_AFTER)
}

/// The earliest moment another fetch may be attempted, or `None` if one may be made now.
pub fn attempt_allowed_at(last_attempt: Option<SystemTime>, now: SystemTime) -> Option<SystemTime> {
    let allowed_at = last_attempt? + ATTEMPT_LOCK;
    (allowed_at > now).then_some(allowed_at)
}

#[derive(Debug)]
pub enum RefreshError {
    /// The last attempt was too recent. Another may be made at the given moment.
    TooSoon(SystemTime),
    /// The attempt could not be recorded, so it was not made: without that record,
    /// nothing would stop the app asking again on every launch.
    CacheUnwritable(io::Error),
    Fetch(FetchError),
}

impl fmt::Display for RefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSoon(_) => write!(f, "the last attempt was too recent"),
            Self::CacheUnwritable(error) => write!(f, "cannot write to the cache: {error}"),
            Self::Fetch(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RefreshError {}

/// Fetches fresh Element Sets from `source` and stores them in `cache`.
///
/// The attempt is recorded before it is made, so even one that fails or is interrupted
/// counts towards [`ATTEMPT_LOCK`]. On any failure the cache keeps what it had.
pub fn refresh(
    source: &dyn Source,
    cache: &Cache,
    now: SystemTime,
) -> Result<Loaded, RefreshError> {
    if let Some(allowed_at) = attempt_allowed_at(cache.last_attempt(), now) {
        return Err(RefreshError::TooSoon(allowed_at));
    }
    cache
        .record_attempt(now)
        .map_err(RefreshError::CacheUnwritable)?;

    let files = source.fetch().map_err(RefreshError::Fetch)?;
    let loaded =
        Loaded::parse(&files, now).ok_or(RefreshError::Fetch(FetchError::NotElementSets))?;

    if let Err(error) = cache.store(&files, now) {
        // The fresh Element Sets are still usable for this run.
        log::warn!("could not store the fetched Element Sets: {error}");
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{StubSource, files, hours};

    #[test]
    fn element_sets_go_stale_after_six_hours() {
        let obtained = SystemTime::UNIX_EPOCH + hours(100);
        assert!(!is_stale(obtained, obtained + hours(5)));
        assert!(is_stale(obtained, obtained + hours(7)));
        // A clock that has gone backwards does not make data stale.
        assert!(!is_stale(obtained, obtained - hours(1)));
    }

    #[test]
    fn attempts_are_locked_for_two_hours() {
        let attempt = SystemTime::UNIX_EPOCH + hours(100);
        assert_eq!(attempt_allowed_at(None, attempt), None);
        assert_eq!(
            attempt_allowed_at(Some(attempt), attempt + hours(1)),
            Some(attempt + hours(2))
        );
        assert_eq!(attempt_allowed_at(Some(attempt), attempt + hours(3)), None);
    }

    #[test]
    fn a_successful_refresh_is_stored_and_locks_further_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let source = StubSource::succeeding(files());
        let now = SystemTime::UNIX_EPOCH + hours(100);

        let loaded = refresh(&source, &cache, now).expect("a successful refresh");
        assert_eq!(loaded.objects.len(), 3);
        assert_eq!(loaded.fetched_at, now);

        let cached = cache.load().expect("stored Element Sets");
        assert_eq!(cached.objects.len(), 3);
        assert_eq!(cached.fetched_at, now);

        let again = refresh(&source, &cache, now + hours(1));
        assert!(matches!(again, Err(RefreshError::TooSoon(at)) if at == now + hours(2)));
        assert_eq!(source.fetches(), 1);
    }

    #[test]
    fn a_failed_refresh_keeps_the_cache_and_still_locks_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let earlier = SystemTime::UNIX_EPOCH + hours(100);
        refresh(&StubSource::succeeding(files()), &cache, earlier).unwrap();

        let failing = StubSource::failing(FetchError::Refused(403));
        let later = earlier + hours(10);
        let result = refresh(&failing, &cache, later);
        assert!(matches!(
            result,
            Err(RefreshError::Fetch(FetchError::Refused(403)))
        ));

        // The earlier Element Sets are untouched, and the failure counts as an attempt.
        assert_eq!(cache.load().unwrap().fetched_at, earlier);
        assert_eq!(cache.last_attempt(), Some(later));
        assert!(matches!(
            refresh(&failing, &cache, later + hours(1)),
            Err(RefreshError::TooSoon(_))
        ));
        assert_eq!(failing.fetches(), 1);
    }

    #[test]
    fn a_response_that_is_not_element_sets_is_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let mut bad = files();
        bad.debris = "OBJECT_NAME,OBJECT_ID\nnot,enough,columns\n".to_owned();

        let result = refresh(
            &StubSource::succeeding(bad),
            &cache,
            SystemTime::UNIX_EPOCH + hours(100),
        );
        assert!(matches!(
            result,
            Err(RefreshError::Fetch(FetchError::NotElementSets))
        ));
        assert!(cache.load().is_none());
    }
}
