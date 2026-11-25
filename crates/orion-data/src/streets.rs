//! Fetching the street map's tiles and keeping them on disk.
//!
//! The tiles are ready-drawn pictures from OpenStreetMap's own server (ADR 0006). It
//! is run on donations and asks three things of an app that uses it: say who is
//! asking, keep every tile for at least a week instead of fetching it again, and
//! never fetch tiles nobody is looking at. See
//! <https://operations.osmfoundation.org/policies/tiles/>.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use orion_core::feature::stable_key;
use orion_core::tiles::TileKey;

use crate::FetchError;

/// Where street tiles are fetched from unless the settings say otherwise: `{z}` is
/// the level, `{x}` the column and `{y}` the row.
pub const DEFAULT_ADDRESS: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";
/// The credit that must be shown whenever the street map is.
pub const CREDIT: &str = "© OpenStreetMap contributors";
/// The deepest level the street map is published at.
pub const MAX_LEVEL: u8 = 19;
/// How long a tile on disk is used before a fresh copy is asked for. The map changes
/// as its volunteers edit it; a month is well over the week the host asks for.
pub const FRESH_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// A tile is a few tens of kilobytes; this guards against a runaway response.
const MAX_TILE_BYTES: u64 = 4 * 1024 * 1024;

/// A host that publishes street tiles without a key.
pub struct StreetTiles {
    agent: ureq::Agent,
    address: String,
}

impl StreetTiles {
    /// `address` is a pattern with `{z}`, `{x}` and `{y}` where the level, column and
    /// row go.
    pub fn new(address: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            // The host asks every app to name itself, so it can tell them apart.
            .user_agent(concat!("orion/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            address: address.into(),
        }
    }

    /// Whether an address has all three of the places a tile's numbers go.
    pub fn is_address(address: &str) -> bool {
        (address.starts_with("https://") || address.starts_with("http://"))
            && ["{z}", "{x}", "{y}"]
                .iter()
                .all(|part| address.contains(part))
    }

    /// The address of one tile.
    pub fn url(&self, key: TileKey) -> String {
        self.address
            .replace("{z}", &key.level.to_string())
            .replace("{x}", &key.col.to_string())
            .replace("{y}", &key.row.to_string())
    }

    /// Fetches one tile's encoded bytes.
    pub fn fetch(&self, key: TileKey) -> Result<Vec<u8>, FetchError> {
        let mut response = self
            .agent
            .get(self.url(key))
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(status) => FetchError::Refused(status),
                other => FetchError::Network(other.to_string()),
            })?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_TILE_BYTES)
            .read_to_vec()
            .map_err(|error| FetchError::Network(error.to_string()))
    }
}

/// Street tiles kept on disk. Tiles from different addresses are kept apart, so
/// changing the address never shows one host's map as another's.
#[derive(Clone, Debug)]
pub struct StreetCache {
    dir: PathBuf,
}

impl StreetCache {
    /// A cache inside `tiles_dir` for tiles fetched from `address`.
    pub fn at(tiles_dir: impl Into<PathBuf>, address: &str) -> Self {
        let folder = format!("streets-{:016x}", stable_key(address));
        Self {
            dir: tiles_dir.into().join(folder),
        }
    }

    fn path(&self, key: TileKey) -> PathBuf {
        self.dir
            .join(key.level.to_string())
            .join(key.col.to_string())
            .join(format!("{}.png", key.row))
    }

    /// The stored tile and whether it is still fresh at `now`, if it was ever fetched.
    pub fn get(&self, key: TileKey, now: SystemTime) -> Option<(Vec<u8>, bool)> {
        let path = self.path(key);
        let bytes = fs::read(&path).ok()?;
        let stored = fs::metadata(&path).and_then(|file| file.modified()).ok()?;
        let fresh = now.duration_since(stored).is_ok_and(|age| age <= FRESH_FOR);
        // A clock set back makes a tile look as if it came from the future: keep it.
        Some((bytes, fresh || stored > now))
    }

    pub fn put(&self, key: TileKey, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(key);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        // Written beside the final name and renamed, so a reader never sees half a tile.
        let partial = path.with_extension("partial");
        fs::write(&partial, bytes)?;
        fs::rename(partial, path)
    }
}

/// Returns a tile's encoded bytes from the cache while it is fresh, or fetches and
/// stores it. A tile gone stale is still better than none, so it is used if the host
/// cannot be reached.
pub fn load_street_tile(
    source: &StreetTiles,
    cache: Option<&StreetCache>,
    key: TileKey,
    now: SystemTime,
) -> Result<Vec<u8>, FetchError> {
    let kept = cache.and_then(|cache| cache.get(key, now));
    if let Some((bytes, true)) = kept {
        return Ok(bytes);
    }
    match source.fetch(key) {
        Ok(bytes) => {
            if let Some(cache) = cache
                && let Err(error) = cache.put(key, &bytes)
            {
                log::warn!("could not store a street map tile: {error}");
            }
            Ok(bytes)
        }
        Err(error) => kept.map(|(bytes, _)| bytes).ok_or(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeServer;

    const PARIS: TileKey = TileKey {
        level: 14,
        row: 5636,
        col: 8299,
    };

    /// A host at the fake server, with the usual pattern after its address.
    fn host(server: &FakeServer) -> (StreetTiles, String) {
        let address = format!(
            "{}/{{z}}/{{x}}/{{y}}.png",
            server.endpoint().trim_end_matches("/gp.php")
        );
        (StreetTiles::new(address.clone()), address)
    }

    #[test]
    fn a_tiles_address_puts_its_level_column_and_row_into_the_pattern() {
        let tiles = StreetTiles::new(DEFAULT_ADDRESS);
        assert_eq!(
            tiles.url(PARIS),
            "https://tile.openstreetmap.org/14/8299/5636.png"
        );
        let other = StreetTiles::new("https://maps.example/tiles?z={z}&row={y}&col={x}");
        assert_eq!(
            other.url(PARIS),
            "https://maps.example/tiles?z=14&row=5636&col=8299"
        );
        assert!(StreetTiles::is_address(DEFAULT_ADDRESS));
        assert!(!StreetTiles::is_address("https://maps.example/{z}/{x}.png"));
        assert!(!StreetTiles::is_address(
            "tile.openstreetmap.org/{z}/{x}/{y}.png"
        ));
    }

    #[test]
    fn the_host_is_told_who_is_asking() {
        let server = FakeServer::start(|_| (200, "tile".to_owned()));
        let (tiles, _) = host(&server);
        tiles.fetch(PARIS).unwrap();
        let request = server.requests().remove(0).to_lowercase();
        assert!(request.contains("user-agent: orion/"), "{request}");
        assert!(request.starts_with("get /14/8299/5636.png "), "{request}");
    }

    #[test]
    fn a_tile_is_fetched_once_and_then_read_from_disk() {
        let server = FakeServer::start(|_| (200, "tile bytes".to_owned()));
        let folder = tempfile::tempdir().unwrap();
        let (tiles, address) = host(&server);
        let cache = StreetCache::at(folder.path(), &address);
        let now = SystemTime::now();

        let first = load_street_tile(&tiles, Some(&cache), PARIS, now).unwrap();
        let second = load_street_tile(&tiles, Some(&cache), PARIS, now).unwrap();
        assert_eq!(first, b"tile bytes");
        assert_eq!(second, first);
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn a_tile_is_kept_a_month_before_a_fresh_copy_is_asked_for() {
        let server = FakeServer::start(|_| (200, "newer".to_owned()));
        let folder = tempfile::tempdir().unwrap();
        let (tiles, address) = host(&server);
        let cache = StreetCache::at(folder.path(), &address);
        cache.put(PARIS, b"older").unwrap();
        let stored = SystemTime::now();

        // Three weeks on it is used as it is: the host asks for at least one.
        let three_weeks = stored + Duration::from_secs(21 * 86_400);
        let kept = load_street_tile(&tiles, Some(&cache), PARIS, three_weeks).unwrap();
        assert_eq!(kept, b"older");
        assert!(server.requests().is_empty());

        // Five weeks on it is fetched again and the new copy kept.
        let five_weeks = stored + Duration::from_secs(35 * 86_400);
        let fresh = load_street_tile(&tiles, Some(&cache), PARIS, five_weeks).unwrap();
        assert_eq!(fresh, b"newer");
        assert_eq!(cache.get(PARIS, SystemTime::now()).unwrap().0, b"newer");
    }

    #[test]
    fn a_stale_tile_is_used_when_the_host_cannot_be_reached_and_a_refusal_is_not_stored() {
        let server = FakeServer::start(|_| (503, "busy".to_owned()));
        let folder = tempfile::tempdir().unwrap();
        let (tiles, address) = host(&server);
        let cache = StreetCache::at(folder.path(), &address);
        let now = SystemTime::now();

        assert_eq!(
            load_street_tile(&tiles, Some(&cache), PARIS, now),
            Err(FetchError::Refused(503))
        );
        assert!(cache.get(PARIS, now).is_none());

        cache.put(PARIS, b"older").unwrap();
        let much_later = now + Duration::from_secs(90 * 86_400);
        let stale = load_street_tile(&tiles, Some(&cache), PARIS, much_later).unwrap();
        assert_eq!(stale, b"older");
    }

    #[test]
    fn tiles_from_different_addresses_are_kept_apart() {
        let folder = tempfile::tempdir().unwrap();
        let one = StreetCache::at(folder.path(), DEFAULT_ADDRESS);
        let other = StreetCache::at(folder.path(), "https://maps.example/{z}/{x}/{y}.png");
        one.put(PARIS, b"one").unwrap();
        let now = SystemTime::now();
        assert!(other.get(PARIS, now).is_none());
        assert_eq!(one.get(PARIS, now).unwrap(), (b"one".to_vec(), true));
    }
}
