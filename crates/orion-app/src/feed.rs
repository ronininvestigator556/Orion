//! Where the app's Element Sets come from: the bundled Snapshot, the cache on disk,
//! or a fresh fetch running in the background.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use orion_core::catalogue::{ObjectKind, TrackedObject, parse_element_set_files};
use orion_data::{Cache, Loaded, RefreshError, Source, attempt_allowed_at, is_stale};

/// The Snapshot: Element Sets shipped inside the app, most trusted kind first.
const SNAPSHOT: [(ObjectKind, &str); 3] = [
    (
        ObjectKind::Satellite,
        include_str!("../assets/snapshot/satellites.csv"),
    ),
    (
        ObjectKind::RocketBody,
        include_str!("../assets/snapshot/rocket_bodies.csv"),
    ),
    (
        ObjectKind::Debris,
        include_str!("../assets/snapshot/debris.csv"),
    ),
];

/// When the Snapshot was fetched from CelesTrak: 25 November 2026, 05:16:24 UTC,
/// in seconds since 1970.
const SNAPSHOT_FETCHED_UNIX: u64 = 1_795_583_784;

fn snapshot_fetched_at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(SNAPSHOT_FETCHED_UNIX)
}

/// Reads the Tracked Objects in the Snapshot.
pub fn load_snapshot() -> anyhow::Result<Vec<TrackedObject>> {
    let parsed = parse_element_set_files(&SNAPSHOT).context("reading the bundled Snapshot")?;
    if parsed.rejected > 0 {
        log::warn!(
            "{} Element Sets in the Snapshot describe orbits the model cannot represent",
            parsed.rejected
        );
    }
    Ok(parsed.objects)
}

/// Where the Element Sets on show came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Snapshot,
    Fetched,
}

/// Whether fresh Element Sets can be fetched right now.
#[derive(Clone, Debug, PartialEq)]
pub enum Refresh {
    Ready,
    InProgress,
    /// The last attempt was too recent; another is allowed after this long.
    Locked(Duration),
    /// The last attempt in this run failed. No more are made until the app is restarted.
    Failed(String),
    /// There is nowhere to keep fetched Element Sets, so none are fetched.
    Unavailable,
}

/// What the interface shows about the Element Sets.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub source: &'static str,
    pub origin: Origin,
    /// Time since the Element Sets on show were fetched from the Source.
    pub age: Duration,
    pub refresh: Refresh,
}

pub struct Feed {
    source: Arc<dyn Source>,
    cache: Option<Cache>,
    origin: Origin,
    fetched_at: SystemTime,
    last_attempt: Option<SystemTime>,
    in_flight: Option<Receiver<Result<Loaded, RefreshError>>>,
    failure: Option<String>,
}

impl Feed {
    /// Chooses the freshest Element Sets available without the network and returns
    /// their Tracked Objects, then starts a refresh in the background if they are stale.
    pub fn start(
        source: Arc<dyn Source>,
        cache: Option<Cache>,
        now: SystemTime,
    ) -> anyhow::Result<(Self, Vec<TrackedObject>)> {
        // A new version of the app can ship a Snapshot newer than an old cache.
        let cached = cache
            .as_ref()
            .and_then(Cache::load)
            .filter(|cached| cached.fetched_at > snapshot_fetched_at());
        let (origin, fetched_at, objects) = match cached {
            Some(cached) => (Origin::Fetched, cached.fetched_at, cached.objects),
            None => (Origin::Snapshot, snapshot_fetched_at(), load_snapshot()?),
        };
        log::info!(
            "showing {} Tracked Objects from {}",
            objects.len(),
            match origin {
                Origin::Snapshot => "the Snapshot",
                Origin::Fetched => "the cache",
            }
        );

        let mut feed = Self {
            last_attempt: cache.as_ref().and_then(Cache::last_attempt),
            source,
            cache,
            origin,
            fetched_at,
            in_flight: None,
            failure: None,
        };
        if is_stale(fetched_at, now) {
            feed.request_refresh(now);
        }
        Ok((feed, objects))
    }

    /// Starts fetching fresh Element Sets in the background, if that is allowed right now.
    pub fn request_refresh(&mut self, now: SystemTime) {
        if self.refresh_availability(now) != Refresh::Ready {
            return;
        }
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let source = Arc::clone(&self.source);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            // The receiver is gone only if the app is shutting down.
            let _ = sender.send(orion_data::refresh(source.as_ref(), &cache, now));
        });
        self.last_attempt = Some(now);
        self.in_flight = Some(receiver);
        log::info!("fetching Element Sets from {}", self.source.name());
    }

    /// Returns freshly fetched Tracked Objects once, when a refresh completes.
    pub fn poll(&mut self) -> Option<Vec<TrackedObject>> {
        let outcome = match self.in_flight.as_ref()?.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(RefreshError::Fetch(
                orion_data::FetchError::Network("the fetch stopped unexpectedly".to_owned()),
            )),
        };
        self.in_flight = None;

        match outcome {
            Ok(loaded) => {
                log::info!(
                    "fetched {} Tracked Objects from {}",
                    loaded.objects.len(),
                    self.source.name()
                );
                self.origin = Origin::Fetched;
                self.fetched_at = loaded.fetched_at;
                Some(loaded.objects)
            }
            Err(error) => {
                log::warn!("refreshing Element Sets failed: {error}");
                self.failure = Some(error.to_string());
                None
            }
        }
    }

    pub fn status(&self, now: SystemTime) -> Status {
        Status {
            source: self.source.name(),
            origin: self.origin,
            age: now.duration_since(self.fetched_at).unwrap_or_default(),
            refresh: self.refresh_availability(now),
        }
    }

    fn refresh_availability(&self, now: SystemTime) -> Refresh {
        if self.in_flight.is_some() {
            Refresh::InProgress
        } else if let Some(failure) = &self.failure {
            Refresh::Failed(failure.clone())
        } else if self.cache.is_none() {
            Refresh::Unavailable
        } else if let Some(allowed_at) = attempt_allowed_at(self.last_attempt, now) {
            Refresh::Locked(allowed_at.duration_since(now).unwrap_or_default())
        } else {
            Refresh::Ready
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use orion_data::{ElementSetFiles, FetchError};

    use super::*;

    fn hours(count: u64) -> Duration {
        Duration::from_secs(count * 60 * 60)
    }

    /// A Source that answers with the Snapshot's own files, or refuses, without any network.
    struct StubSource {
        refuse: bool,
        fetches: AtomicUsize,
    }

    impl StubSource {
        fn new(refuse: bool) -> Arc<Self> {
            Arc::new(Self {
                refuse,
                fetches: AtomicUsize::new(0),
            })
        }

        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    impl Source for StubSource {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn fetch(&self) -> Result<ElementSetFiles, FetchError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if self.refuse {
                return Err(FetchError::Refused(403));
            }
            let [(_, satellites), (_, rocket_bodies), (_, debris)] = SNAPSHOT;
            Ok(ElementSetFiles {
                satellites: satellites.to_owned(),
                rocket_bodies: rocket_bodies.to_owned(),
                debris: debris.to_owned(),
            })
        }
    }

    /// Polls until the refresh in flight finishes, returning any Tracked Objects it delivered.
    fn wait_for_refresh(feed: &mut Feed, now: SystemTime) -> Option<Vec<TrackedObject>> {
        for _ in 0..1_000 {
            if let Some(objects) = feed.poll() {
                return Some(objects);
            }
            if feed.status(now).refresh != Refresh::InProgress {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the refresh did not finish");
    }

    #[test]
    fn a_fresh_snapshot_is_shown_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let source = StubSource::new(false);
        let now = snapshot_fetched_at() + hours(1);

        let (feed, objects) =
            Feed::start(source.clone(), Some(Cache::at(dir.path())), now).unwrap();

        assert!(objects.len() > 29_000);
        let status = feed.status(now);
        assert_eq!(status.origin, Origin::Snapshot);
        assert_eq!(status.age, hours(1));
        assert_eq!(status.refresh, Refresh::Ready);
        assert_eq!(source.fetches(), 0);
    }

    #[test]
    fn a_stale_snapshot_is_refreshed_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let source = StubSource::new(false);
        let now = snapshot_fetched_at() + hours(7);

        let (mut feed, _) = Feed::start(source.clone(), Some(cache.clone()), now).unwrap();
        assert_eq!(feed.status(now).refresh, Refresh::InProgress);

        let objects = wait_for_refresh(&mut feed, now).expect("fetched Tracked Objects");
        assert!(objects.len() > 29_000);

        let later = now + hours(1);
        let status = feed.status(later);
        assert_eq!(status.origin, Origin::Fetched);
        assert_eq!(status.age, hours(1));
        assert_eq!(status.refresh, Refresh::Locked(hours(1)));

        // Asking again inside the lock does nothing.
        feed.request_refresh(later);
        assert_eq!(source.fetches(), 1);
        assert!(cache.load().is_some());
    }

    #[test]
    fn a_failed_refresh_stops_further_attempts_for_this_run() {
        let dir = tempfile::tempdir().unwrap();
        let source = StubSource::new(true);
        let now = snapshot_fetched_at() + hours(7);

        let (mut feed, objects) =
            Feed::start(source.clone(), Some(Cache::at(dir.path())), now).unwrap();
        assert!(wait_for_refresh(&mut feed, now).is_none());

        // The Snapshot stays on show, and even after the lock expires nothing is retried.
        assert!(!objects.is_empty());
        let much_later = now + hours(5);
        let status = feed.status(much_later);
        assert_eq!(status.origin, Origin::Snapshot);
        assert!(matches!(status.refresh, Refresh::Failed(_)), "{status:?}");
        feed.request_refresh(much_later);
        assert_eq!(source.fetches(), 1);
    }

    #[test]
    fn a_cache_newer_than_the_snapshot_is_preferred() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at(dir.path());
        let fetched_at = snapshot_fetched_at() + hours(30);
        orion_data::refresh(StubSource::new(false).as_ref(), &cache, fetched_at).unwrap();

        let source = StubSource::new(false);
        let now = fetched_at + hours(3);
        let (feed, _) = Feed::start(source.clone(), Some(cache), now).unwrap();

        let status = feed.status(now);
        assert_eq!(status.origin, Origin::Fetched);
        assert_eq!(status.age, hours(3));
        assert_eq!(source.fetches(), 0);
    }

    #[test]
    fn without_a_cache_nothing_is_fetched() {
        let source = StubSource::new(false);
        let now = snapshot_fetched_at() + hours(7);

        let (mut feed, _) = Feed::start(source.clone(), None, now).unwrap();
        assert_eq!(feed.status(now).refresh, Refresh::Unavailable);
        feed.request_refresh(now);
        assert_eq!(source.fetches(), 0);
    }
}
