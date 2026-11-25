//! Where Element Sets are fetched from.

use std::fmt;
use std::time::Duration;

use orion_core::filter::Group;

use crate::{ElementSetFiles, has_catalogue_header, has_element_set_header};

/// A place Element Sets can be fetched from.
pub trait Source: Send + Sync {
    /// Name shown to the user.
    fn name(&self) -> &'static str;

    /// Fetches Element Sets for every kind of Tracked Object.
    ///
    /// An implementation must stop at the first failure and not retry: the caller
    /// decides when another attempt is allowed.
    fn fetch(&self) -> Result<ElementSetFiles, FetchError>;

    /// Fetches the catalogue file that Catalogue Records are read from.
    fn fetch_catalogue(&self) -> Result<String, FetchError>;

    /// Fetches a file listing the members of one of the Groups in [`Group::LISTED`].
    fn fetch_group(&self, group: Group) -> Result<String, FetchError>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    /// The Source answered with this HTTP status instead of data.
    Refused(u16),
    /// The Source could not be reached, or the transfer failed.
    Network(String),
    /// The Source answered, but not with Element Sets.
    NotElementSets,
    /// The Source answered, but not with a catalogue file.
    NotCatalogueRecords,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(403) => {
                write!(
                    f,
                    "the source refused the request (403); it may have been asked too recently"
                )
            }
            Self::Refused(status) => write!(f, "the source answered with HTTP status {status}"),
            Self::Network(reason) => write!(f, "could not reach the source: {reason}"),
            Self::NotElementSets => write!(f, "the source did not return Element Sets"),
            Self::NotCatalogueRecords => {
                write!(f, "the source did not return Catalogue Records")
            }
        }
    }
}

impl std::error::Error for FetchError {}

/// Far above any real file (the largest is under 3 MB); guards against a runaway response.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// CelesTrak's public Element Set service.
///
/// CelesTrak has no single query for the whole Catalogue. Three queries cover about
/// nine tenths of it: the active Satellites group, and name searches for rocket bodies
/// and debris. Dead Satellites belong to no group and are missing.
pub struct CelesTrak {
    agent: ureq::Agent,
    endpoint: String,
    catalogue_endpoint: String,
}

impl CelesTrak {
    pub const ENDPOINT: &'static str = "https://celestrak.org/NORAD/elements/gp.php";

    /// The whole satellite catalogue as one file, about 7 MB.
    pub const CATALOGUE_ENDPOINT: &'static str = "https://celestrak.org/pub/satcat.csv";

    pub fn new() -> Self {
        Self::with_endpoints(Self::ENDPOINT, Self::CATALOGUE_ENDPOINT)
    }

    /// Uses different addresses for the service, for testing against a local server.
    pub fn with_endpoints(
        endpoint: impl Into<String>,
        catalogue_endpoint: impl Into<String>,
    ) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(concat!("orion/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.into(),
            endpoint: endpoint.into(),
            catalogue_endpoint: catalogue_endpoint.into(),
        }
    }

    fn query(&self, key: &str, value: &str) -> Result<String, FetchError> {
        let mut response = self
            .agent
            .get(&self.endpoint)
            .query(key, value)
            // CelesTrak asks clients to use CSV: it is a third the size of JSON.
            .query("FORMAT", "csv")
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(status) => FetchError::Refused(status),
                other => FetchError::Network(other.to_string()),
            })?;
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| FetchError::Network(error.to_string()))?;

        // Errors can arrive as a plain-text message with a success status.
        if has_element_set_header(&text) {
            Ok(text)
        } else {
            Err(FetchError::NotElementSets)
        }
    }
}

impl Default for CelesTrak {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for CelesTrak {
    fn name(&self) -> &'static str {
        "CelesTrak"
    }

    fn fetch(&self) -> Result<ElementSetFiles, FetchError> {
        // `?` returns at the first failure, so nothing more is asked after an error.
        Ok(ElementSetFiles {
            satellites: self.query("GROUP", "active")?,
            rocket_bodies: self.query("NAME", "R/B")?,
            debris: self.query("NAME", "DEB")?,
        })
    }

    fn fetch_group(&self, group: Group) -> Result<String, FetchError> {
        let name = match group {
            Group::Weather => "weather",
            Group::SpaceStations => "stations",
            Group::Navigation => "gnss",
            // Constellations are recognised by name, so CelesTrak is never asked for them.
            Group::Starlink | Group::OneWeb | Group::Kuiper | Group::Qianfan | Group::Guowang => {
                return Err(FetchError::NotElementSets);
            }
        };
        self.query("GROUP", name)
    }

    fn fetch_catalogue(&self) -> Result<String, FetchError> {
        let mut response = self
            .agent
            .get(&self.catalogue_endpoint)
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(status) => FetchError::Refused(status),
                other => FetchError::Network(other.to_string()),
            })?;
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| FetchError::Network(error.to_string()))?;

        if has_catalogue_header(&text) {
            Ok(text)
        } else {
            Err(FetchError::NotCatalogueRecords)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeServer, HEADER, catalogue, files};

    fn celestrak(server: &FakeServer) -> CelesTrak {
        CelesTrak::with_endpoints(server.endpoint(), server.catalogue_endpoint())
    }

    #[test]
    fn listed_groups_are_asked_for_by_celestraks_names() {
        let server = FakeServer::start(|_| (200, files().satellites));
        let source = celestrak(&server);
        for group in Group::LISTED {
            assert_eq!(source.fetch_group(group), Ok(files().satellites));
        }
        let requests = server.requests();
        for (request, name) in requests.iter().zip(["weather", "stations", "gnss"]) {
            assert!(request.contains(&format!("GROUP={name}&")), "{request}");
        }

        // A constellation is never asked for.
        assert!(source.fetch_group(Group::Starlink).is_err());
        assert_eq!(server.requests().len(), 3);
    }

    #[test]
    fn the_catalogue_is_fetched_from_its_own_address() {
        let server = FakeServer::start(|_| (200, catalogue()));
        assert_eq!(celestrak(&server).fetch_catalogue(), Ok(catalogue()));

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with("GET /pub/satcat.csv "),
            "{}",
            requests[0]
        );
    }

    #[test]
    fn a_catalogue_answer_that_is_something_else_is_rejected() {
        let server = FakeServer::start(|_| (200, files().satellites));
        assert_eq!(
            celestrak(&server).fetch_catalogue(),
            Err(FetchError::NotCatalogueRecords)
        );

        let refusing = FakeServer::start(|_| (403, "no".to_owned()));
        assert_eq!(
            celestrak(&refusing).fetch_catalogue(),
            Err(FetchError::Refused(403))
        );
    }

    fn body_for(request: &str) -> String {
        let files = files();
        if request.contains("GROUP=active") {
            files.satellites
        } else if request.contains("NAME=R%2FB") || request.contains("NAME=R/B") {
            files.rocket_bodies
        } else if request.contains("NAME=DEB") {
            files.debris
        } else {
            panic!("unexpected request: {request}");
        }
    }

    #[test]
    fn fetch_asks_for_each_kind_as_csv() {
        let server = FakeServer::start(|request| (200, body_for(request)));
        let fetched = celestrak(&server).fetch().expect("a successful fetch");
        assert_eq!(fetched, files());

        let requests = server.requests();
        assert_eq!(requests.len(), 3);
        for request in &requests {
            assert!(request.contains("FORMAT=csv"), "{request}");
            assert!(
                request.to_lowercase().contains("user-agent: orion/"),
                "{request}"
            );
        }
    }

    #[test]
    fn fetch_stops_at_the_first_refusal() {
        let server = FakeServer::start(|request| {
            if request.contains("GROUP=active") {
                (200, body_for(request))
            } else {
                (
                    403,
                    "GP data has not updated since your last successful download".to_owned(),
                )
            }
        });
        let result = celestrak(&server).fetch();

        assert_eq!(result, Err(FetchError::Refused(403)));
        // The refused request was the second; the third was never sent.
        assert_eq!(server.requests().len(), 2);
    }

    #[test]
    fn a_message_with_a_success_status_is_not_element_sets() {
        let server = FakeServer::start(|_| (200, "No GP data found".to_owned()));
        let result = celestrak(&server).fetch();

        assert_eq!(result, Err(FetchError::NotElementSets));
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn an_unreachable_source_is_a_network_error() {
        // Nothing listens on port 1.
        let result =
            CelesTrak::with_endpoints("http://127.0.0.1:1/gp.php", "http://127.0.0.1:1/satcat.csv")
                .fetch();
        assert!(matches!(result, Err(FetchError::Network(_))), "{result:?}");
    }

    #[test]
    fn the_header_constant_matches_real_files() {
        assert!(has_element_set_header(HEADER));
    }
}
