//! Fetching Base Map tiles and keeping them on disk.

use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use orion_core::tiles::TileKey;

use crate::FetchError;

/// One picture of the world, published as a pyramid of tiles.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TileLayer {
    /// The name the service knows the picture by.
    pub name: &'static str,
    /// Which moment's picture to ask for, as a date or a date and time; `default`
    /// for pictures that do not change.
    pub time: Cow<'static, str>,
    /// The name of the tile grid the picture is cut on.
    pub matrix_set: &'static str,
    /// File extension, which also says how the tile is encoded.
    pub extension: &'static str,
    /// The deepest level the picture is published at.
    pub max_level: u8,
}

/// NASA's Blue Marble: the whole Earth, cloud-free, at about 500 metres per pixel.
pub const BLUE_MARBLE: TileLayer = TileLayer {
    name: "BlueMarble_ShadedRelief_Bathymetry",
    time: Cow::Borrowed("default"),
    matrix_set: "500m",
    extension: "jpeg",
    max_level: 7,
};

/// A cloud-free Landsat composite of the year 2000, at about 30 metres per pixel.
pub const LANDSAT: TileLayer = TileLayer {
    name: "Landsat_WELD_CorrectedReflectance_TrueColor_Global_Annual",
    time: Cow::Borrowed("2000-12-01"),
    matrix_set: "31.25m",
    extension: "jpeg",
    max_level: 11,
};

impl TileLayer {
    /// The same picture as it was at another moment, written the way the service
    /// expects: `2026-11-24` for a daily picture, `2026-11-25T06:50:00Z` for one made
    /// several times a day.
    pub fn at(&self, time: impl Into<String>) -> Self {
        Self {
            time: Cow::Owned(time.into()),
            ..self.clone()
        }
    }
}

/// A picture made afresh each day or several times a day. Its `time` here is only a
/// placeholder: ask for a moment with [`TileLayer::at`].
const fn dated(
    name: &'static str,
    matrix_set: &'static str,
    extension: &'static str,
    max_level: u8,
) -> TileLayer {
    TileLayer {
        name,
        time: Cow::Borrowed("default"),
        matrix_set,
        extension,
        max_level,
    }
}

/// The day's view of the whole Earth in true colour, clouds and all, from the VIIRS
/// instrument on the NOAA-20 satellite. One picture a day.
pub const DAILY_TRUE_COLOUR: TileLayer = dated(
    "VIIRS_NOAA20_CorrectedReflectance_TrueColor",
    "250m",
    "jpeg",
    8,
);

/// The Americas and the Atlantic from the GOES-East weather satellite, in colour by
/// day and infrared by night. A picture every ten minutes.
pub const GOES_EAST: TileLayer = dated("GOES-East_ABI_GeoColor", "1km", "png", 6);

/// The Pacific and western Americas from the GOES-West weather satellite, likewise.
pub const GOES_WEST: TileLayer = dated("GOES-West_ABI_GeoColor", "1km", "png", 6);

/// East Asia, Australia and the western Pacific from the Himawari weather satellite,
/// in infrared, where the coldest and so highest cloud tops are brightest.
pub const HIMAWARI: TileLayer = dated("Himawari_AHI_Band13_Clean_Infrared", "2km", "png", 5);

/// How hard it rained or snowed, estimated from many satellites together by NASA's
/// IMERG. One picture a day, published a couple of days later.
pub const PRECIPITATION: TileLayer = dated("IMERG_Precipitation_Rate", "2km", "png", 5);

/// The temperature of the sea's surface, from many instruments combined. One picture
/// a day, published a couple of days later.
pub const SEA_SURFACE_TEMPERATURE: TileLayer =
    dated("GHRSST_L4_MUR_Sea_Surface_Temperature", "1km", "png", 6);

/// A tile is a few tens of kilobytes; this guards against a runaway response.
const MAX_TILE_BYTES: u64 = 8 * 1024 * 1024;

/// NASA's Global Imagery Browse Services, which publishes tiles without a key.
pub struct TileSource {
    agent: ureq::Agent,
    endpoint: String,
}

impl TileSource {
    pub const ENDPOINT: &'static str = "https://gibs.earthdata.nasa.gov/wmts/epsg4326/best";

    pub fn new() -> Self {
        Self::with_endpoint(Self::ENDPOINT)
    }

    /// Uses a different address for the service, for testing against a local server.
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("orion/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            endpoint: endpoint.into(),
        }
    }

    /// The address of one tile.
    pub fn url(&self, layer: &TileLayer, key: TileKey) -> String {
        format!(
            "{}/{}/default/{}/{}/{}/{}/{}.{}",
            self.endpoint,
            layer.name,
            layer.time,
            layer.matrix_set,
            key.level,
            key.row,
            key.col,
            layer.extension
        )
    }

    /// Fetches one tile's encoded bytes.
    pub fn fetch(&self, layer: &TileLayer, key: TileKey) -> Result<Vec<u8>, FetchError> {
        let mut response =
            self.agent
                .get(self.url(layer, key))
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

impl Default for TileSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Tiles kept on disk, so each is fetched from the service only once.
#[derive(Clone, Debug)]
pub struct TileCache {
    dir: PathBuf,
}

impl TileCache {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, layer: &TileLayer, key: TileKey) -> PathBuf {
        self.dir
            .join(layer.name)
            // A colon cannot be part of a folder's name everywhere.
            .join(layer.time.replace(':', "-"))
            .join(key.level.to_string())
            .join(key.row.to_string())
            .join(format!("{}.{}", key.col, layer.extension))
    }

    /// The stored tile, if it has been fetched before.
    pub fn get(&self, layer: &TileLayer, key: TileKey) -> Option<Vec<u8>> {
        fs::read(self.path(layer, key)).ok()
    }

    pub fn put(&self, layer: &TileLayer, key: TileKey, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(layer, key);
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        // Written beside the final name and renamed, so a reader never sees half a tile.
        let partial = path.with_extension("partial");
        fs::write(&partial, bytes)?;
        fs::rename(partial, path)
    }
}

impl TileCache {
    /// The most disk space tiles may take before the oldest are cleared out.
    pub const DEFAULT_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

    /// Deletes the tiles stored longest ago until those left take no more than nine
    /// tenths of `limit_bytes`, if together they take more than the limit. Returns how
    /// many bytes were freed. Meant to be run once at launch, off the main thread.
    pub fn trim(&self, limit_bytes: u64) -> u64 {
        let mut files = Vec::new();
        collect_files(&self.dir, &mut files);
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        if total <= limit_bytes {
            return 0;
        }
        let before = total;
        files.sort_by_key(|(stored, ..)| *stored);
        for (_, size, path) in files {
            if total <= limit_bytes / 10 * 9 {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
        before - total
    }
}

/// Lists every file under `dir` with when it was stored and its size.
fn collect_files(dir: &Path, files: &mut Vec<(SystemTime, u64, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_files(&entry.path(), files);
        } else if metadata.is_file() {
            let stored = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((stored, metadata.len(), entry.path()));
        }
    }
}

/// Returns a tile's encoded bytes from the cache, or fetches and stores it.
pub fn load_tile(
    source: &TileSource,
    cache: Option<&TileCache>,
    layer: &TileLayer,
    key: TileKey,
) -> Result<Vec<u8>, FetchError> {
    if let Some(bytes) = cache.and_then(|cache| cache.get(layer, key)) {
        return Ok(bytes);
    }
    let bytes = source.fetch(layer, key)?;
    if let Some(cache) = cache
        && let Err(error) = cache.put(layer, key, &bytes)
    {
        log::warn!("could not store a Base Map tile: {error}");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeServer;

    const KEY: TileKey = TileKey {
        level: 9,
        row: 92,
        col: 102,
    };

    fn source(server: &FakeServer) -> TileSource {
        // The fake server's address ends in a file name; tiles hang off its folder.
        let endpoint = server.endpoint().trim_end_matches("/gp.php").to_owned();
        TileSource::with_endpoint(endpoint)
    }

    #[test]
    fn tile_addresses_follow_the_services_pattern() {
        let source = TileSource::new();
        assert_eq!(
            source.url(&LANDSAT, KEY),
            "https://gibs.earthdata.nasa.gov/wmts/epsg4326/best/\
             Landsat_WELD_CorrectedReflectance_TrueColor_Global_Annual/default/2000-12-01/31.25m/9/92/102.jpeg"
        );
        assert!(
            source
                .url(
                    &BLUE_MARBLE,
                    TileKey {
                        level: 3,
                        row: 1,
                        col: 1
                    }
                )
                .ends_with("/BlueMarble_ShadedRelief_Bathymetry/default/default/500m/3/1/1.jpeg")
        );
    }

    #[test]
    fn a_dated_picture_is_asked_for_by_its_moment_and_kept_apart_by_it() {
        let source = TileSource::new();
        let morning = GOES_EAST.at("2026-11-25T06:50:00Z");
        assert!(
            source
                .url(&morning, KEY)
                .ends_with("/GOES-East_ABI_GeoColor/default/2026-11-25T06:50:00Z/1km/9/92/102.png")
        );
        assert_eq!(GOES_EAST.time, "default");

        let dir = tempfile::tempdir().unwrap();
        let cache = TileCache::at(dir.path());
        cache.put(&morning, KEY, b"morning").unwrap();
        assert_eq!(cache.get(&GOES_EAST.at("2026-11-25T07:00:00Z"), KEY), None);
        assert_eq!(cache.get(&morning, KEY).unwrap(), b"morning");
    }

    #[test]
    fn a_tile_is_fetched_once_and_then_read_from_disk() {
        let server = FakeServer::start(|_| (200, "tile bytes".to_owned()));
        let dir = tempfile::tempdir().unwrap();
        let cache = TileCache::at(dir.path());
        let source = source(&server);

        let first = load_tile(&source, Some(&cache), &LANDSAT, KEY).unwrap();
        let second = load_tile(&source, Some(&cache), &LANDSAT, KEY).unwrap();
        assert_eq!(first, b"tile bytes");
        assert_eq!(second, first);

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with("GET /Landsat_WELD_CorrectedReflectance_TrueColor_Global_Annual/default/2000-12-01/31.25m/9/92/102.jpeg "),
            "{}",
            requests[0]
        );
        assert!(requests[0].to_lowercase().contains("user-agent: orion/"));
    }

    #[test]
    fn tiles_of_different_pictures_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TileCache::at(dir.path());
        cache.put(&LANDSAT, KEY, b"landsat").unwrap();
        assert_eq!(cache.get(&BLUE_MARBLE, KEY), None);
        cache.put(&BLUE_MARBLE, KEY, b"blue marble").unwrap();

        assert_eq!(cache.get(&LANDSAT, KEY).unwrap(), b"landsat");
        assert_eq!(cache.get(&BLUE_MARBLE, KEY).unwrap(), b"blue marble");
    }

    #[test]
    fn a_refused_tile_is_an_error_and_is_not_stored() {
        let server = FakeServer::start(|_| (404, "no such tile".to_owned()));
        let dir = tempfile::tempdir().unwrap();
        let cache = TileCache::at(dir.path());

        let result = load_tile(&source(&server), Some(&cache), &LANDSAT, KEY);
        assert_eq!(result, Err(FetchError::Refused(404)));
        assert_eq!(cache.get(&LANDSAT, KEY), None);
    }

    #[test]
    fn without_a_cache_every_load_asks_the_service() {
        let server = FakeServer::start(|_| (200, "tile bytes".to_owned()));
        let source = source(&server);
        load_tile(&source, None, &BLUE_MARBLE, KEY).unwrap();
        load_tile(&source, None, &BLUE_MARBLE, KEY).unwrap();
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn trimming_clears_the_oldest_tiles_once_over_the_limit() {
        let folder = tempfile::tempdir().unwrap();
        let cache = TileCache::at(folder.path());
        let keys: Vec<TileKey> = (0..10)
            .map(|col| TileKey {
                level: 5,
                row: 3,
                col,
            })
            .collect();
        for (age, key) in keys.iter().enumerate() {
            cache.put(&BLUE_MARBLE, *key, &[0; 100]).unwrap();
            // Make the order they were stored in unmistakable: the first is the oldest.
            let stored = SystemTime::now() - Duration::from_secs(1000 - age as u64 * 10);
            let file = fs::File::options()
                .write(true)
                .open(cache.path(&BLUE_MARBLE, *key))
                .unwrap();
            file.set_modified(stored).unwrap();
        }

        // Under the limit nothing is touched.
        assert_eq!(cache.trim(1000), 0);
        assert!(cache.get(&BLUE_MARBLE, keys[0]).is_some());

        // Over it, the oldest go until nine tenths of the limit is left: 500 bytes
        // allows 450, which is four tiles.
        assert_eq!(cache.trim(500), 600);
        assert!(cache.get(&BLUE_MARBLE, keys[5]).is_none());
        assert!(cache.get(&BLUE_MARBLE, keys[6]).is_some());
        assert!(cache.get(&BLUE_MARBLE, keys[9]).is_some());
    }

    #[test]
    fn trimming_a_cache_that_was_never_written_does_nothing() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(TileCache::at(folder.path().join("missing")).trim(0), 0);
    }
}
