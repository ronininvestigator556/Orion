//! Listings: single files a Source republishes every so often, each listing the Features
//! of one Layer, such as the week's earthquakes.

use std::fmt;
use std::time::{Duration, SystemTime};

use orion_core::feature::Feature;

use crate::{Cache, FetchError};

/// Why a listing's file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the source's answer could not be read: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// One file from one Source, and how to read it.
#[derive(Clone, Copy, Debug)]
pub struct Listing {
    /// Who publishes it, as shown to the user.
    pub source: &'static str,
    pub url: &'static str,
    /// The kind of answer to ask for, where the Source needs telling.
    pub accept: Option<&'static str>,
    /// The name its file is kept under in the cache folder.
    pub cache_name: &'static str,
    pub parse: fn(&str) -> Result<Vec<Feature>, ParseError>,
    /// How long a copy stays fresh. The Source is not asked again before then unless
    /// the user asks.
    pub refresh_every: Duration,
}

/// Why a listing could not be loaded.
#[derive(Clone, Debug, PartialEq)]
pub enum ListingError {
    Fetch(FetchError),
    Parse(ParseError),
}

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ListingError {}

/// Something that can fetch a file over the network.
pub trait Fetcher: Send + Sync {
    fn get(&self, url: &str, accept: Option<&str>) -> Result<String, FetchError>;
}

/// Far above any listing (the largest is a couple of megabytes); guards against a
/// runaway response.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// Fetches over HTTP, naming Orion and its version and nothing about the user.
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("orion/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for Http {
    fn get(&self, url: &str, accept: Option<&str>) -> Result<String, FetchError> {
        let mut request = self.agent.get(url);
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        // Finland's Digitraffic asks every app to name itself in a header of its own.
        if url.contains("digitraffic.fi/") {
            request = request.header("Digitraffic-User", "Orion");
        }
        let mut response = request.call().map_err(|error| match error {
            ureq::Error::StatusCode(status) => FetchError::Refused(status),
            other => FetchError::Network(other.to_string()),
        })?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| FetchError::Network(error.to_string()))
    }
}

/// The copy of a listing kept from an earlier fetch, with when it was fetched.
pub fn cached_listing(cache: &Cache, listing: &Listing) -> Option<(Vec<Feature>, SystemTime)> {
    let (text, fetched_at) = cache.layer_text(listing.cache_name)?;
    match (listing.parse)(&text) {
        Ok(features) => Some((features, fetched_at)),
        Err(error) => {
            log::warn!(
                "the stored copy of {} is unreadable: {error}",
                listing.cache_name
            );
            None
        }
    }
}

/// Fetches a listing from its Source and reads it. A file that can be read is kept in
/// the cache, when there is one, for the next launch.
pub fn fetch_listing(
    fetcher: &dyn Fetcher,
    cache: Option<&Cache>,
    listing: &Listing,
) -> Result<Vec<Feature>, ListingError> {
    let text = fetcher
        .get(listing.url, listing.accept)
        .map_err(ListingError::Fetch)?;
    let features = (listing.parse)(&text).map_err(ListingError::Parse)?;
    if let Some(cache) = cache
        && let Err(error) = cache.store_layer_text(listing.cache_name, &text)
    {
        log::warn!("could not store {}: {error}", listing.cache_name);
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::FakeServer;

    fn parse_lines(text: &str) -> Result<Vec<Feature>, ParseError> {
        if !text.starts_with("places") {
            return Err(ParseError("no heading".to_owned()));
        }
        Ok(text
            .lines()
            .skip(1)
            .map(|name| Feature::at(name, name, 0.0, 0.0))
            .collect())
    }

    const FEED: Listing = Listing {
        source: "Test",
        url: "",
        accept: Some("text/x-test"),
        cache_name: "test.txt",
        parse: parse_lines,
        refresh_every: Duration::from_secs(60),
    };

    #[test]
    fn a_fetched_listing_is_read_and_kept_for_next_time() {
        let server = FakeServer::start(|_| (200, "places\nA\nB".to_owned()));
        let folder = tempfile::tempdir().unwrap();
        let cache = Cache::at(folder.path());
        let listing = Listing {
            url: Box::leak(server.endpoint().to_owned().into_boxed_str()),
            ..FEED
        };
        assert!(cached_listing(&cache, &listing).is_none());

        let features = fetch_listing(&Http::new(), Some(&cache), &listing).unwrap();
        assert_eq!(features.len(), 2);
        let request = server.requests()[0].to_lowercase();
        assert!(request.contains("accept: text/x-test"), "{request}");
        assert!(request.contains("user-agent: orion/"), "{request}");

        let (kept, fetched_at) = cached_listing(&cache, &listing).unwrap();
        assert_eq!(kept, features);
        assert!(fetched_at.elapsed().unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn an_answer_that_cannot_be_read_is_an_error_and_is_not_kept() {
        let server = FakeServer::start(|_| (200, "<html>busy</html>".to_owned()));
        let folder = tempfile::tempdir().unwrap();
        let cache = Cache::at(folder.path());
        let listing = Listing {
            url: Box::leak(server.endpoint().to_owned().into_boxed_str()),
            ..FEED
        };
        let error = fetch_listing(&Http::new(), Some(&cache), &listing).unwrap_err();
        assert!(matches!(error, ListingError::Parse(_)), "{error}");
        assert!(cached_listing(&cache, &listing).is_none());
    }

    #[test]
    fn a_refusal_is_an_error() {
        let server = FakeServer::start(|_| (503, "later".to_owned()));
        let listing = Listing {
            url: Box::leak(server.endpoint().to_owned().into_boxed_str()),
            ..FEED
        };
        assert_eq!(
            fetch_listing(&Http::new(), None, &listing),
            Err(ListingError::Fetch(FetchError::Refused(503)))
        );
    }
}
