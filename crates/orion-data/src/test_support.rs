//! Shared helpers for this crate's tests.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use orion_core::filter::Group;

use crate::{ElementSetFiles, FetchError, Source};

pub const HEADER: &str = "OBJECT_NAME,OBJECT_ID,EPOCH,MEAN_MOTION,ECCENTRICITY,INCLINATION,RA_OF_ASC_NODE,ARG_OF_PERICENTER,MEAN_ANOMALY,EPHEMERIS_TYPE,CLASSIFICATION_TYPE,NORAD_CAT_ID,ELEMENT_SET_NO,REV_AT_EPOCH,BSTAR,MEAN_MOTION_DOT,MEAN_MOTION_DDOT";

pub fn hours(count: u64) -> Duration {
    Duration::from_secs(count * 60 * 60)
}

/// One real Element Set of each kind, from CelesTrak on 24 November 2026.
pub fn files() -> ElementSetFiles {
    let file = |row: &str| format!("{HEADER}\n{row}\n");
    ElementSetFiles {
        satellites: file(
            "CALSPHERE 1,1964-063C,2026-11-24T19:13:23.305152,13.76776729,.00275414,90.2245,75.5043,163.9822,294.5252,0,U,900,999,9352,.4264642E-3,.43E-5,0",
        ),
        rocket_bodies: file(
            "VANGUARD R/B,1959-001B,2026-11-24T13:42:50.850432,11.48745088,.16436474,32.9106,19.8578,234.7966,108.7631,0,U,12,999,54977,.38946772E-3,.697E-5,0",
        ),
        debris: file(
            "ECHO 1 DEB [METAL OBJ],1960-009C,2026-11-24T21:05:33.060768,12.18387137,.01100896,47.2139,272.1129,315.3207,43.8752,0,U,51,999,95076,.51388874E-4,-.79E-6,0",
        ),
    }
}

/// Two real rows from CelesTrak's satellite catalogue. The first has decayed.
pub fn catalogue() -> String {
    "OBJECT_NAME,OBJECT_ID,NORAD_CAT_ID,OBJECT_TYPE,OPS_STATUS_CODE,OWNER,LAUNCH_DATE,LAUNCH_SITE,DECAY_DATE,PERIOD,INCLINATION,APOGEE,PERIGEE,RCS,DATA_STATUS_CODE,ORBIT_CENTER,ORBIT_TYPE\n\
     SPUTNIK 1,1957-001B,2,PAY,D,CIS,1957-10-04,TYMSC,1958-01-03,96.10,65.00,1080,64,,,EA,IMP\n\
     ISS (ZARYA),1998-067A,25544,PAY,+,ISS,1998-11-20,TYMSC,,92.93,51.63,423,413,399.0524,,EA,ORB\n"
        .to_owned()
}

/// A Source that answers from memory and counts how often it is asked.
pub struct StubSource {
    answer: Result<ElementSetFiles, FetchError>,
    catalogue_answer: Result<String, FetchError>,
    group_answer: Result<String, FetchError>,
    fetches: AtomicUsize,
    catalogue_fetches: AtomicUsize,
    group_fetches: AtomicUsize,
}

/// A member list holding one object, as every Group's answer.
fn group_members() -> String {
    "NORAD_CAT_ID\n25544\n".to_owned()
}

impl StubSource {
    pub fn succeeding(files: ElementSetFiles) -> Self {
        Self {
            answer: Ok(files),
            catalogue_answer: Ok(catalogue()),
            group_answer: Ok(group_members()),
            fetches: AtomicUsize::new(0),
            catalogue_fetches: AtomicUsize::new(0),
            group_fetches: AtomicUsize::new(0),
        }
    }

    pub fn failing(error: FetchError) -> Self {
        Self {
            answer: Err(error.clone()),
            catalogue_answer: Err(error.clone()),
            group_answer: Err(error),
            fetches: AtomicUsize::new(0),
            catalogue_fetches: AtomicUsize::new(0),
            group_fetches: AtomicUsize::new(0),
        }
    }

    /// Changes what the Source answers when asked for the catalogue file.
    pub fn with_catalogue(mut self, answer: Result<String, FetchError>) -> Self {
        self.catalogue_answer = answer;
        self
    }

    /// Changes what the Source answers when asked for any Group's members.
    pub fn with_groups(mut self, answer: Result<String, FetchError>) -> Self {
        self.group_answer = answer;
        self
    }

    pub fn group_fetches(&self) -> usize {
        self.group_fetches.load(Ordering::SeqCst)
    }

    pub fn catalogue_fetches(&self) -> usize {
        self.catalogue_fetches.load(Ordering::SeqCst)
    }

    pub fn fetches(&self) -> usize {
        self.fetches.load(Ordering::SeqCst)
    }
}

impl Source for StubSource {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn fetch(&self) -> Result<ElementSetFiles, FetchError> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }

    fn fetch_catalogue(&self) -> Result<String, FetchError> {
        self.catalogue_fetches.fetch_add(1, Ordering::SeqCst);
        self.catalogue_answer.clone()
    }

    fn fetch_group(&self, _group: Group) -> Result<String, FetchError> {
        self.group_fetches.fetch_add(1, Ordering::SeqCst);
        self.group_answer.clone()
    }
}

/// A minimal HTTP server on this machine that answers every request with whatever
/// `respond` returns for it, and remembers the requests it received.
pub struct FakeServer {
    endpoint: String,
    catalogue_endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    /// `respond` receives the request line and headers and returns a status and body.
    pub fn start(respond: impl Fn(&str) -> (u16, String) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free local port");
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}/gp.php");
        let catalogue_endpoint = format!("http://{address}/pub/satcat.csv");
        let requests = Arc::new(Mutex::new(Vec::new()));

        let recorded = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };

                // Read up to the blank line that ends the headers.
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                let request = String::from_utf8_lossy(&request).into_owned();

                let (status, body) = respond(&request);
                recorded.lock().unwrap().push(request);
                let response = format!(
                    "HTTP/1.1 {status} Status\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });

        Self {
            endpoint,
            catalogue_endpoint,
            requests,
        }
    }

    pub fn catalogue_endpoint(&self) -> &str {
        &self.catalogue_endpoint
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
