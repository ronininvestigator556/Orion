//! Fetching and caching Element Sets for Orion.
//!
//! Element Sets come from a [`Source`] on the network and are kept in a [`Cache`] on
//! disk, so the app can start without a connection. [`refresh`] ties the two together
//! and enforces how often a Source may be asked.

pub mod advisories;
mod cache;
pub mod hazards;
pub mod import;
pub mod listing;
pub mod places;
pub mod roads;
pub mod settings;
mod source;
pub mod streets;
#[cfg(test)]
mod test_support;
pub mod tiles;
pub mod traffic;
pub mod unrest;

use std::fmt;
use std::io;
use std::time::{Duration, SystemTime};

use orion_core::catalogue::{
    CatalogueRecords, ObjectKind, TrackedObject, parse_catalogue_records_csv,
    parse_element_set_files,
};

use orion_core::filter::{Group, GroupLists, parse_group_members_csv};

pub use cache::Cache;
pub use source::{CelesTrak, FetchError, Source};

/// Element Sets older than this are refreshed automatically.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);

/// Minimum time between two attempts to fetch, whether or not the first succeeded.
///
/// CelesTrak updates its data every two hours and blocks addresses that ask more often
/// or that keep asking after an error, so this is deliberately strict.
pub const ATTEMPT_LOCK: Duration = Duration::from_secs(2 * 60 * 60);

/// Catalogue Records older than this are fetched again alongside the Element Sets.
///
/// Owners, launch dates and launch sites never change for an object already listed, so
/// the only cost of an old copy is that the newest launches show no record for a while.
/// The file is several megabytes, so it is fetched far less often than Element Sets.
pub const CATALOGUE_STALE_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

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
    /// Catalogue Records, when they were fetched along with the Element Sets.
    pub catalogue: Option<CatalogueRecords>,
    /// Group member lists, when they were fetched along with the Catalogue Records.
    pub groups: Option<GroupLists>,
}

/// First column of every CSV Element Set file.
const CSV_HEADER_START: &str = "OBJECT_NAME,";

/// True if the text begins like a CSV Element Set file. Without this check, arbitrary
/// text would pass as a file with a strange header and no rows.
fn has_element_set_header(text: &str) -> bool {
    text.starts_with(CSV_HEADER_START)
}

/// True if the text begins like CelesTrak's satellite catalogue in CSV form.
fn has_catalogue_header(text: &str) -> bool {
    let header = text.lines().next().unwrap_or_default();
    header.contains("NORAD_CAT_ID") && header.contains("LAUNCH_SITE")
}

/// Reads Catalogue Records, or `None` if the text is not a usable catalogue file.
fn parse_catalogue(text: &str) -> Option<CatalogueRecords> {
    if !has_catalogue_header(text) {
        return None;
    }
    parse_catalogue_records_csv(text)
        .ok()
        .filter(|records| !records.is_empty())
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
            catalogue: None,
            groups: None,
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
    let mut loaded =
        Loaded::parse(&files, now).ok_or(RefreshError::Fetch(FetchError::NotElementSets))?;

    if let Err(error) = cache.store(&files, now) {
        // The fresh Element Sets are still usable for this run.
        log::warn!("could not store the fetched Element Sets: {error}");
    }

    let catalogue_is_due = cache
        .catalogue_fetched_at()
        .is_none_or(|fetched_at| is_older_than(fetched_at, now, CATALOGUE_STALE_AFTER));
    if catalogue_is_due {
        loaded.catalogue = refresh_catalogue(source, cache, now);
        // Group lists change about as rarely, so they ride along. After a failure
        // nothing more is asked of the source.
        if loaded.catalogue.is_some() {
            loaded.groups = refresh_groups(source, cache);
        }
    }
    Ok(loaded)
}

/// Fetches and stores Catalogue Records. A failure here is not a failed refresh: the
/// Element Sets are what matter, and the records on hand stay in use.
fn refresh_catalogue(
    source: &dyn Source,
    cache: &Cache,
    now: SystemTime,
) -> Option<CatalogueRecords> {
    let text = match source.fetch_catalogue() {
        Ok(text) => text,
        Err(error) => {
            log::warn!("could not fetch Catalogue Records: {error}");
            return None;
        }
    };
    let Some(records) = parse_catalogue(&text) else {
        log::warn!("the source did not return Catalogue Records");
        return None;
    };
    if let Err(error) = cache.store_catalogue(&text, now) {
        log::warn!("could not store the fetched Catalogue Records: {error}");
    }
    Some(records)
}

/// Fetches and stores the member list of every listed Group, all or nothing.
fn refresh_groups(source: &dyn Source, cache: &Cache) -> Option<GroupLists> {
    let mut lists = GroupLists::default();
    let mut texts = Vec::new();
    for group in Group::LISTED {
        let text = match source.fetch_group(group) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("could not fetch the {group:?} group: {error}");
                return None;
            }
        };
        let Some(members) = parse_group(&text) else {
            log::warn!("the source did not return a member list for the {group:?} group");
            return None;
        };
        lists.set(group, members);
        texts.push((group, text));
    }
    for (group, text) in &texts {
        if let Err(error) = cache.store_group(*group, text) {
            log::warn!("could not store the {group:?} group: {error}");
        }
    }
    Some(lists)
}

/// Reads a Group's members, or `None` if the text is not a usable member list.
fn parse_group(text: &str) -> Option<std::collections::HashSet<u32>> {
    parse_group_members_csv(text)
        .ok()
        .filter(|members| !members.is_empty())
}

fn is_older_than(obtained_at: SystemTime, now: SystemTime, limit: Duration) -> bool {
    // A time in the future means the clock was changed; treat the data as fresh.
    now.duration_since(obtained_at).is_ok_and(|age| age > limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{StubSource, catalogue, files, hours};

    #[test]
    fn group_lists_come_with_the_catalogue_records() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let source = StubSource::succeeding(files());
        let start = SystemTime::UNIX_EPOCH + hours(100);

        let first = refresh(&source, &cache, start).unwrap();
        let lists = first.groups.expect("group lists");
        for group in Group::LISTED {
            assert_eq!(lists.len(group), 1, "{group:?}");
        }
        assert_eq!(cache.load_groups(), Some(lists));
        assert_eq!(source.group_fetches(), 3);

        // Like the records, they are left alone until a week has passed.
        assert!(
            refresh(&source, &cache, start + hours(24))
                .unwrap()
                .groups
                .is_none()
        );
        assert_eq!(source.group_fetches(), 3);
    }

    #[test]
    fn a_group_that_cannot_be_fetched_stops_the_rest_but_not_the_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let source = StubSource::succeeding(files()).with_groups(Err(FetchError::Refused(403)));

        let loaded = refresh(&source, &cache, SystemTime::UNIX_EPOCH + hours(100)).unwrap();
        assert!(loaded.catalogue.is_some());
        assert!(loaded.groups.is_none());
        assert!(cache.load_groups().is_none());
        // Asked once, refused, and not asked again for the other two.
        assert_eq!(source.group_fetches(), 1);
    }

    #[test]
    fn no_groups_are_asked_for_when_the_catalogue_records_fail() {
        let dir = tempfile::tempdir().unwrap();
        let source = StubSource::succeeding(files()).with_catalogue(Err(FetchError::Refused(403)));
        refresh(
            &source,
            &Cache::at(dir.path()),
            SystemTime::UNIX_EPOCH + hours(100),
        )
        .unwrap();
        assert_eq!(source.group_fetches(), 0);
    }

    #[test]
    fn catalogue_records_come_with_the_first_refresh_and_then_weekly() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let source = StubSource::succeeding(files());
        let start = SystemTime::UNIX_EPOCH + hours(100);

        let first = refresh(&source, &cache, start).unwrap();
        assert_eq!(first.catalogue.expect("Catalogue Records").len(), 1);
        let (stored, stored_at) = cache.load_catalogue().expect("stored Catalogue Records");
        assert_eq!(stored[&25_544].owner, "ISS");
        assert_eq!(stored_at, start);

        // A day later the Element Sets are refreshed but the records are left alone.
        let next_day = refresh(&source, &cache, start + hours(24)).unwrap();
        assert!(next_day.catalogue.is_none());
        assert_eq!(source.catalogue_fetches(), 1);

        // After more than a week they are fetched again.
        let next_week = refresh(&source, &cache, start + hours(8 * 24)).unwrap();
        assert!(next_week.catalogue.is_some());
        assert_eq!(source.catalogue_fetches(), 2);
    }

    #[test]
    fn failing_to_get_catalogue_records_does_not_fail_the_refresh() {
        for answer in [
            Err(FetchError::Refused(403)),
            Ok("this is not a catalogue file".to_owned()),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let cache = Cache::at(dir.path());
            let source = StubSource::succeeding(files()).with_catalogue(answer);

            let loaded = refresh(&source, &cache, SystemTime::UNIX_EPOCH + hours(100))
                .expect("the Element Sets still arrive");
            assert_eq!(loaded.objects.len(), 3);
            assert!(loaded.catalogue.is_none());
            assert!(cache.load_catalogue().is_none());
        }
    }

    #[test]
    fn no_catalogue_records_are_asked_for_after_element_sets_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let source = StubSource::failing(FetchError::Refused(403));
        let _ = refresh(
            &source,
            &Cache::at(dir.path()),
            SystemTime::UNIX_EPOCH + hours(100),
        );
        assert_eq!(source.catalogue_fetches(), 0);

        // `catalogue()` is a usable file, so the stub's default answer is realistic.
        assert!(parse_catalogue(&catalogue()).is_some());
    }

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
