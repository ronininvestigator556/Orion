//! The flights Layer: aircraft in the air around the middle of the view, as heard by
//! adsb.lol's volunteer receivers.
//!
//! Unlike a Layer of places, its Features move and its data depends on where the view
//! is. Aircraft are fetched for a circle around the view, again every few minutes and
//! when the view has moved on, and between fetches each is moved along its last
//! reported heading at its last reported speed.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

use glam::Vec3;
use orion_core::layer::{CardRow, FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::listing::{Fetcher, ListingError};
use orion_data::traffic::{
    AIRCRAFT_SOURCE, Aircraft, AircraftKind, MAX_RADIUS_NM, aircraft_url, parse_aircraft, snap,
};

use crate::feed::Refresh;
use crate::layer::{Highlight, Layer, LayerStatus, LayerView};
use crate::lines::Segment;
use crate::objects::{Instance, Marks};
use crate::ui::format_duration;

/// How long the aircraft on hand stay fresh while the view stays put.
const REFRESH_EVERY: Duration = Duration::from_secs(180);
/// The least time between two fetches, however fast the view moves or the user asks.
const MIN_INTERVAL: Duration = Duration::from_secs(30);
/// How long after a failed fetch the next is tried.
const RETRY_AFTER: Duration = Duration::from_secs(120);
/// The longest an aircraft is moved along its last heading before it is left where
/// that puts it. Past this the estimate is worth little.
const MAX_ESTIMATE_SECONDS: f64 = 300.0;
/// How many seconds of travel the line behind each aircraft shows.
const TAIL_SECONDS: f64 = 60.0;
const EARTH_RADIUS_M: f64 = orion_core::EARTH_EQUATORIAL_RADIUS_KM * 1000.0;
const METRES_PER_FOOT: f64 = 0.3048;
const METRES_PER_NAUTICAL_MILE: f64 = 1852.0;

/// The classes of the filter, in order, with their colours as written for the screen.
const CLASSES: [(&str, [u8; 3], bool); 5] = [
    ("Airliners and large aircraft", [198, 255, 0], true),
    ("Light aircraft", [174, 234, 0], true),
    ("Helicopters", [118, 255, 3], true),
    ("Other and unknown", [178, 255, 89], true),
    ("On the ground", [158, 158, 158], false),
];
const ON_GROUND: usize = 4;

fn class_of(aircraft: &Aircraft) -> usize {
    if aircraft.on_ground {
        return ON_GROUND;
    }
    match aircraft.kind {
        AircraftKind::Large => 0,
        AircraftKind::Light => 1,
        AircraftKind::Helicopter => 2,
        AircraftKind::Other => 3,
    }
}

fn linear(color: [u8; 3]) -> [u8; 3] {
    color.map(|channel| ((f32::from(channel) / 255.0).powf(2.2) * 255.0).round() as u8)
}

/// What an aircraft is called in lists and beside its dot.
fn label(aircraft: &Aircraft) -> String {
    let name = [
        &aircraft.callsign,
        &aircraft.registration,
        &aircraft.address,
    ]
    .into_iter()
    .find(|text| !text.is_empty())
    .cloned()
    .unwrap_or_default();
    if aircraft.type_code.is_empty() {
        name
    } else {
        format!("{name} ({})", aircraft.type_code)
    }
}

/// Where an aircraft is `seconds` after its position was heard, in Earth radii in the
/// Earth-fixed frame: moved along its heading at its speed, and at its height.
fn position_after(aircraft: &Aircraft, seconds: f64) -> Vec3 {
    let (mut latitude, mut longitude) = (
        aircraft.latitude_deg.to_radians(),
        aircraft.longitude_deg.to_radians(),
    );
    if let (false, Some(speed), Some(track)) =
        (aircraft.on_ground, aircraft.speed_kt, aircraft.track_deg)
    {
        let seconds = seconds.clamp(-MAX_ESTIMATE_SECONDS, MAX_ESTIMATE_SECONDS);
        let distance = speed * METRES_PER_NAUTICAL_MILE / 3600.0 * seconds / EARTH_RADIUS_M;
        let bearing = track.to_radians();
        // The destination a given distance along a great circle from a start point.
        let moved = (latitude.sin() * distance.cos()
            + latitude.cos() * distance.sin() * bearing.cos())
        .clamp(-1.0, 1.0)
        .asin();
        longitude += (bearing.sin() * distance.sin() * latitude.cos())
            .atan2(distance.cos() - latitude.sin() * moved.sin());
        latitude = moved;
    }
    let height = aircraft.altitude_ft.unwrap_or(0.0).max(0.0) * METRES_PER_FOOT;
    let radius = 1.0 + height / EARTH_RADIUS_M;
    Vec3::new(
        (radius * latitude.cos() * longitude.cos()) as f32,
        (radius * latitude.cos() * longitude.sin()) as f32,
        (radius * latitude.sin()) as f32,
    )
}

/// A number with a comma between thousands, such as `12,825`.
fn grouped(number: f64) -> String {
    let digits = format!("{:.0}", number.abs());
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            text.push(',');
        }
        text.push(digit);
    }
    if number < 0.0 && text != "0" {
        text.insert(0, '-');
    }
    text
}

type Fetched = Result<Vec<Aircraft>, ListingError>;

pub struct FlightLayer {
    fetcher: Arc<dyn Fetcher>,
    on: bool,
    aircraft: Vec<Aircraft>,
    labels: Vec<String>,
    /// Each aircraft's callsign, registration and type in lower case, for searching.
    names: Vec<String>,
    class_on: [bool; CLASSES.len()],
    search: String,
    listed: Vec<bool>,
    listed_count: usize,
    /// Where each aircraft is estimated to be now.
    positions: Vec<Vec3>,
    dots: Vec<Instance>,
    /// The whole-degree place the view is over.
    view: Option<(i32, i32)>,
    /// The whole-degree place the aircraft on hand were fetched around.
    region: Option<(i32, i32)>,
    fetched_at: Option<SystemTime>,
    in_flight: Option<(Receiver<Fetched>, (i32, i32))>,
    last_attempt: Option<SystemTime>,
    failure: Option<String>,
}

impl FlightLayer {
    /// A Layer that is off and has fetched nothing.
    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            fetcher,
            on: false,
            aircraft: Vec::new(),
            labels: Vec::new(),
            names: Vec::new(),
            class_on: CLASSES.map(|(_, _, on)| on),
            search: String::new(),
            listed: Vec::new(),
            listed_count: 0,
            positions: Vec::new(),
            dots: Vec::new(),
            view: None,
            region: None,
            fetched_at: None,
            in_flight: None,
            last_attempt: None,
            failure: None,
        }
    }

    fn set_aircraft(&mut self, aircraft: Vec<Aircraft>, region: (i32, i32), now: SystemTime) {
        self.labels = aircraft.iter().map(label).collect();
        self.names = aircraft
            .iter()
            .map(|aircraft| {
                format!(
                    "{} {} {} {}",
                    aircraft.callsign, aircraft.registration, aircraft.type_code, aircraft.address
                )
                .to_lowercase()
            })
            .collect();
        self.aircraft = aircraft;
        self.region = Some(region);
        self.fetched_at = Some(now);
        self.refilter();
    }

    fn refilter(&mut self) {
        self.listed = self
            .aircraft
            .iter()
            .zip(&self.names)
            .map(|(aircraft, name)| {
                self.class_on[class_of(aircraft)] && name.contains(&self.search)
            })
            .collect();
        self.listed_count = self.listed.iter().filter(|listed| **listed).count();
    }

    /// Takes in a fetch that has finished, and starts one if the aircraft on hand are
    /// stale or were fetched for somewhere the view has left.
    fn tick(&mut self, now: SystemTime) {
        if !self.on {
            return;
        }
        if let Some((receiver, region)) = &self.in_flight {
            let region = *region;
            match receiver.try_recv() {
                Ok(Ok(aircraft)) => {
                    self.in_flight = None;
                    self.failure = None;
                    self.set_aircraft(aircraft, region, now);
                }
                Ok(Err(error)) => {
                    self.in_flight = None;
                    log::warn!("fetching flights: {error}");
                    self.failure = Some(error.to_string());
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.in_flight = None;
                    self.failure = Some("the fetch stopped unexpectedly".to_owned());
                }
            }
        }
        let Some(view) = self.view else {
            return;
        };
        let since = |earlier: Option<SystemTime>| {
            earlier.map(|earlier| now.duration_since(earlier).unwrap_or_default())
        };
        let wait = if self.failure.is_some() {
            RETRY_AFTER
        } else {
            MIN_INTERVAL
        };
        let may_try = since(self.last_attempt).is_none_or(|ago| ago >= wait);
        let moved_on = self.region != Some(view);
        let stale = since(self.fetched_at).is_none_or(|age| age >= REFRESH_EVERY);
        if may_try && (moved_on || stale) {
            self.start_fetch(view, now);
        }
    }

    fn start_fetch(&mut self, region: (i32, i32), now: SystemTime) {
        if self.in_flight.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let fetcher = Arc::clone(&self.fetcher);
        let url = aircraft_url(f64::from(region.0), f64::from(region.1), MAX_RADIUS_NM);
        std::thread::spawn(move || {
            let fetched = fetcher
                .get(&url, None)
                .map_err(ListingError::Fetch)
                .and_then(|text| parse_aircraft(&text).map_err(ListingError::Parse));
            // The receiver is gone only if the app is shutting down.
            let _ = sender.send(fetched);
        });
        self.in_flight = Some((receiver, region));
        self.last_attempt = Some(now);
    }

    /// How long ago each aircraft's position was heard, less its own delay.
    fn seconds_since_fetch(&self, now: SystemTime) -> f64 {
        self.fetched_at
            .map(|at| now.duration_since(at).unwrap_or_default().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn rebuild(&mut self, now: SystemTime, highlight: Highlight) {
        let elapsed = self.seconds_since_fetch(now);
        self.positions = self
            .aircraft
            .iter()
            .map(|aircraft| position_after(aircraft, elapsed + aircraft.position_age_s))
            .collect();
        let colors = CLASSES.map(|(_, color, _)| linear(color));
        self.dots = self
            .aircraft
            .iter()
            .enumerate()
            .map(|(index, aircraft)| {
                if !self.listed[index] {
                    return Instance::HIDDEN;
                }
                let [r, g, b] = colors[class_of(aircraft)];
                let marks = Marks {
                    selected: highlight.selected == Some(index),
                    hovered: highlight.hovered == Some(index),
                    stale: false,
                };
                let scale = match aircraft.kind {
                    AircraftKind::Large => 1.8,
                    _ => 1.3,
                };
                Instance::new(self.positions[index], [r, g, b, 255], marks).scaled(scale)
            })
            .collect();
    }
}

impl Layer for FlightLayer {
    fn name(&self) -> &'static str {
        "Flights"
    }

    fn theme(&self) -> Theme {
        Theme::Traffic
    }

    fn time_coverage(&self) -> TimeCoverage {
        TimeCoverage::PresentOnly
    }

    fn is_on(&self) -> bool {
        self.on
    }

    fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.dots.clear();
        }
    }

    fn swatch(&self) -> [u8; 4] {
        let [r, g, b] = linear(CLASSES[0].1);
        [r, g, b, 255]
    }

    fn counts(&self) -> (usize, usize) {
        (self.listed_count, self.aircraft.len())
    }

    fn status(&self, now: SystemTime) -> LayerStatus {
        let age = self
            .fetched_at
            .map(|at| format_duration(now.duration_since(at).unwrap_or_default()));
        let mut text = match (&self.failure, &age) {
            (Some(reason), Some(age)) => format!(
                "Could not reach {AIRCRAFT_SOURCE}: {reason}. Showing what was heard {age} ago; trying again shortly."
            ),
            (Some(reason), None) => {
                format!("Could not reach {AIRCRAFT_SOURCE}: {reason}. Trying again shortly.")
            }
            (None, Some(age)) => format!("From {AIRCRAFT_SOURCE}, heard {age} ago."),
            (None, None) => format!("Fetching from {AIRCRAFT_SOURCE}."),
        };
        text.push_str(
            " Aircraft within about 460 km of the middle of the view, which is all the \
             service is told about, to the nearest degree. Between refreshes each is \
             moved along its last heading.",
        );
        let since_attempt = self
            .last_attempt
            .map(|at| now.duration_since(at).unwrap_or_default());
        let refresh = match since_attempt {
            _ if self.in_flight.is_some() => Refresh::InProgress,
            Some(ago) if ago < MIN_INTERVAL => Refresh::Locked(MIN_INTERVAL - ago),
            _ => Refresh::Ready,
        };
        LayerStatus {
            text,
            refresh: Some(refresh),
        }
    }

    fn request_refresh(&mut self, now: SystemTime) {
        let recently = self
            .last_attempt
            .is_some_and(|at| now.duration_since(at).unwrap_or_default() < MIN_INTERVAL);
        if let (true, false, Some(view)) = (self.on, recently, self.view) {
            self.failure = None;
            self.start_fetch(view, now);
        }
    }

    fn set_search(&mut self, text: &str) {
        let text = text.trim().to_lowercase();
        if text != self.search {
            self.search = text;
            self.refilter();
        }
    }

    fn found(&self, limit: usize) -> Vec<(usize, String)> {
        self.labels
            .iter()
            .enumerate()
            .filter(|(index, _)| self.listed[*index])
            .take(limit)
            .map(|(index, label)| (index, label.clone()))
            .collect()
    }

    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        for ((label, [r, g, b], _), on) in CLASSES.iter().zip(&mut self.class_on) {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(on, "").changed();
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(swatch, 5.0, egui::Color32::from_rgb(*r, *g, *b));
                ui.label(*label);
            });
        }
        if changed {
            self.refilter();
        }
    }

    fn set_view(&mut self, view: &LayerView) {
        self.view = Some(snap(view.latitude_deg, view.longitude_deg));
    }

    fn update(&mut self, _at: JulianDate, highlight: Highlight) {
        let now = SystemTime::now();
        self.tick(now);
        self.rebuild(now, highlight);
    }

    fn dots(&self) -> &[Instance] {
        &self.dots
    }

    /// A short line behind each aircraft, showing where it has come from.
    fn lines(&self, _selected: Option<usize>, _at: JulianDate, out: &mut Vec<Segment>) {
        let elapsed = self.seconds_since_fetch(SystemTime::now());
        let colors = CLASSES.map(|(_, color, _)| linear(color).map(|c| f32::from(c) / 255.0));
        for (index, aircraft) in self.aircraft.iter().enumerate() {
            if !self.listed[index] || aircraft.on_ground || aircraft.speed_kt.is_none() {
                continue;
            }
            let Some(head) = self.positions.get(index) else {
                continue;
            };
            let since_heard = (elapsed + aircraft.position_age_s).min(MAX_ESTIMATE_SECONDS);
            let tail = position_after(aircraft, since_heard - TAIL_SECONDS);
            let [r, g, b] = colors[class_of(aircraft)];
            out.push(Segment::new(tail, *head, [r, g, b, 0.45]));
        }
    }

    fn feature_name(&self, index: usize) -> Option<&str> {
        self.labels.get(index).map(String::as_str)
    }

    fn card(&self, index: usize, _at: JulianDate) -> Option<FeatureCard> {
        let aircraft = self.aircraft.get(index)?;
        let mut rows = Vec::new();
        let mut row = |label: &str, value: String| {
            if !value.trim().is_empty() {
                rows.push(CardRow::new(label, value));
            }
        };
        row("Callsign", aircraft.callsign.clone());
        row("Registration", aircraft.registration.clone());
        row("Type", aircraft.type_code.clone());
        row("Kind", CLASSES[class_of(aircraft)].0.to_owned());
        match (aircraft.on_ground, aircraft.altitude_ft) {
            (true, _) => row("Altitude", "On the ground".to_owned()),
            (false, Some(feet)) => row(
                "Altitude",
                format!(
                    "{} ft ({} m)",
                    grouped(feet),
                    grouped(feet * METRES_PER_FOOT)
                ),
            ),
            (false, None) => {}
        }
        if let Some(knots) = aircraft.speed_kt {
            row(
                "Ground speed",
                format!("{knots:.0} kt ({:.0} km/h)", knots * 1.852),
            );
        }
        if let Some(track) = aircraft.track_deg {
            row("Heading", format!("{track:.0}°"));
        }
        match aircraft.climb_fpm {
            Some(rate) if rate >= 100.0 => row("Climbing", format!("{} ft/min", grouped(rate))),
            Some(rate) if rate <= -100.0 => {
                row("Descending", format!("{} ft/min", grouped(-rate)));
            }
            _ => {}
        }
        row("Squawk", aircraft.squawk.clone());
        row("Transponder address", aircraft.address.clone());

        let heard = self.seconds_since_fetch(SystemTime::now()) + aircraft.position_age_s;
        let notices = aircraft
            .emergency
            .iter()
            .map(|declared| format!("Declaring an emergency: {declared}."))
            .collect();
        Some(FeatureCard {
            title: self.labels.get(index).cloned().unwrap_or_default(),
            sections: vec![rows],
            details: vec![format!(
                "Position last heard {heard:.0} seconds ago and estimated since from its speed and heading."
            )],
            notices,
            source: AIRCRAFT_SOURCE.to_owned(),
        })
    }

    fn stable_key(&self, index: usize) -> Option<u64> {
        self.aircraft.get(index).map(|aircraft| aircraft.key)
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        self.aircraft
            .iter()
            .position(|aircraft| aircraft.key == key)
    }

    fn is_loading(&self) -> bool {
        self.on && self.fetched_at.is_none() && self.failure.is_none()
    }

    fn attribution(&self) -> Option<&'static str> {
        Some("Flights: adsb.lol contributors")
    }

    fn position(&self, index: usize) -> Option<Vec3> {
        self.positions.get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use orion_data::FetchError;

    use super::*;

    /// Answers every fetch with the same thing and remembers what was asked for.
    struct Stub {
        answer: Result<String, FetchError>,
        asked: Mutex<Vec<String>>,
    }

    impl Stub {
        fn answering(answer: Result<&str, FetchError>) -> Arc<Self> {
            Arc::new(Self {
                answer: answer.map(str::to_owned),
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Fetcher for Stub {
        fn get(&self, url: &str, _accept: Option<&str>) -> Result<String, FetchError> {
            self.asked.lock().unwrap().push(url.to_owned());
            self.answer.clone()
        }
    }

    const ANSWER: &str = r#"{"ac":[
        {"hex":"4caa41","flight":"EIN142  ","r":"EI-EIN","t":"A333","alt_baro":35000,"gs":480.0,
         "track":90.0,"baro_rate":-1024,"squawk":"3607","emergency":"none","category":"A5",
         "lat":0.0,"lon":10.0,"seen_pos":0.0},
        {"hex":"406abc","flight":"GABCD   ","r":"G-ABCD","t":"C172","alt_baro":"ground","gs":4.0,
         "track":120.0,"category":"A1","lat":51.47,"lon":-0.46,"seen_pos":2.0},
        {"hex":"43c001","alt_baro":1500,"gs":90,"track":10,"category":"A7","emergency":"general",
         "lat":51.0,"lon":0.1,"seen_pos":0.0}]}"#;

    fn looking_at(layer: &mut FlightLayer, latitude_deg: f64, longitude_deg: f64) {
        layer.set_view(&LayerView {
            latitude_deg,
            longitude_deg,
            ..LayerView::default()
        });
    }

    fn settle(layer: &mut FlightLayer, now: SystemTime) {
        for _ in 0..500 {
            layer.tick(now);
            if layer.in_flight.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the fetch never finished");
    }

    fn started(stub: &Arc<Stub>) -> (FlightLayer, SystemTime) {
        let mut layer = FlightLayer::new(Arc::clone(stub) as Arc<dyn Fetcher>);
        let now = SystemTime::now();
        looking_at(&mut layer, 51.47, -0.46);
        layer.set_on(true);
        settle(&mut layer, now);
        (layer, now)
    }

    #[test]
    fn nothing_is_asked_until_the_layer_is_on_and_then_only_roughly_where() {
        let stub = Stub::answering(Ok(ANSWER));
        let mut layer = FlightLayer::new(Arc::clone(&stub) as Arc<dyn Fetcher>);
        looking_at(&mut layer, 51.4706, -0.461941);
        layer.tick(SystemTime::now());
        assert!(stub.asked().is_empty());
        assert!(!layer.is_loading());

        layer.set_on(true);
        assert!(layer.is_loading());
        settle(&mut layer, SystemTime::now());
        assert_eq!(stub.asked(), ["https://api.adsb.lol/v2/point/51/0/250"]);
        // The one on the ground is filtered out until asked for.
        assert_eq!(layer.counts(), (2, 3));
    }

    #[test]
    fn aircraft_are_fetched_again_when_stale_or_when_the_view_moves_on() {
        let stub = Stub::answering(Ok(ANSWER));
        let (mut layer, now) = started(&stub);

        // The view staying put, nothing more is asked for three minutes.
        layer.tick(now + Duration::from_secs(170));
        assert_eq!(stub.asked().len(), 1);
        settle(&mut layer, now + Duration::from_secs(181));
        assert_eq!(stub.asked().len(), 2);

        // A small move within the same degree changes nothing.
        looking_at(&mut layer, 51.3, -0.2);
        layer.tick(now + Duration::from_secs(230));
        assert_eq!(stub.asked().len(), 2);

        // A move to somewhere else is followed, but not within half a minute of the last fetch.
        looking_at(&mut layer, 40.6, -73.8);
        layer.tick(now + Duration::from_secs(200));
        assert_eq!(stub.asked().len(), 2);
        settle(&mut layer, now + Duration::from_secs(212));
        assert_eq!(stub.asked()[2], "https://api.adsb.lol/v2/point/41/-74/250");
    }

    #[test]
    fn between_fetches_aircraft_move_along_their_heading() {
        let stub = Stub::answering(Ok(ANSWER));
        let (mut layer, now) = started(&stub);

        // Heading due east along the equator at 480 knots: 8 nautical miles a minute,
        // which is 8 minutes of arc, 0.1333 degrees of longitude.
        layer.rebuild(now + Duration::from_secs(60), Highlight::default());
        let position = layer.position(0).unwrap();
        let longitude = position.y.atan2(position.x).to_degrees();
        assert!((longitude - 10.1333).abs() < 0.001, "{longitude}");
        assert!(position.z.abs() < 1e-5);
        // And it is 35,000 feet up: 10.7 km, a six-hundredth of Earth's radius.
        assert!(
            (position.length() - 1.001_673).abs() < 1e-5,
            "{}",
            position.length()
        );

        // The one on the ground stays where it was heard, on the surface.
        let parked = layer.position(1).unwrap();
        assert!((parked.length() - 1.0).abs() < 1e-6);
        let before = parked;
        layer.rebuild(now + Duration::from_secs(120), Highlight::default());
        assert_eq!(layer.position(1), Some(before));

        // An estimate is not pushed past five minutes: an hour on, the airliner is
        // where five minutes would put it.
        layer.rebuild(now + Duration::from_secs(3600), Highlight::default());
        let position = layer.position(0).unwrap();
        let longitude = position.y.atan2(position.x).to_degrees();
        // 40 nautical miles, a shade under 40 minutes of arc at the equator's radius.
        assert!((longitude - 10.6655).abs() < 0.001, "{longitude}");
    }

    #[test]
    fn each_aircraft_trails_a_line_and_shows_a_card() {
        let stub = Stub::answering(Ok(ANSWER));
        let (mut layer, now) = started(&stub);
        layer.class_on[ON_GROUND] = true;
        layer.refilter();
        layer.rebuild(now, Highlight::default());

        // Two in the air trail a line; the one on the ground does not.
        let mut lines = Vec::new();
        layer.lines(None, JulianDate::J2000, &mut lines);
        assert_eq!(lines.len(), 2);

        let card = layer.card(0, JulianDate::J2000).unwrap();
        assert_eq!(card.title, "EIN142 (A333)");
        let value = |label: &str| {
            card.sections[0]
                .iter()
                .find(|row| row.label == label)
                .map(|row| row.value.as_str())
        };
        assert_eq!(value("Altitude"), Some("35,000 ft (10,668 m)"));
        assert_eq!(value("Ground speed"), Some("480 kt (889 km/h)"));
        assert_eq!(value("Heading"), Some("90°"));
        assert_eq!(value("Descending"), Some("1,024 ft/min"));
        assert_eq!(card.source, "adsb.lol");
        assert!(card.notices.is_empty());

        let parked = layer.card(1, JulianDate::J2000).unwrap();
        assert_eq!(
            parked.sections[0][4],
            CardRow::new("Altitude", "On the ground")
        );

        // No callsign or registration: the transponder's address names it.
        let helicopter = layer.card(2, JulianDate::J2000).unwrap();
        assert_eq!(helicopter.title, "43c001");
        assert_eq!(helicopter.notices, ["Declaring an emergency: general."]);
    }

    #[test]
    fn the_search_finds_aircraft_by_callsign_registration_or_type() {
        let stub = Stub::answering(Ok(ANSWER));
        let (mut layer, _) = started(&stub);
        layer.set_search("ein");
        assert_eq!(layer.found(10), [(0, "EIN142 (A333)".to_owned())]);
        layer.set_search("a333");
        assert_eq!(layer.counts(), (1, 3));
        layer.set_search("");
        assert_eq!(layer.counts(), (2, 3));
        let key = layer.stable_key(2).unwrap();
        assert_eq!(layer.index_of(key), Some(2));
    }

    #[test]
    fn a_failure_is_reported_and_tried_again_after_a_while() {
        let stub = Stub::answering(Err(FetchError::Refused(429)));
        let (mut layer, now) = started(&stub);
        assert_eq!(stub.asked().len(), 1);
        assert!(
            layer
                .status(now)
                .text
                .starts_with("Could not reach adsb.lol")
        );
        assert!(!layer.is_loading());

        layer.tick(now + Duration::from_secs(60));
        assert_eq!(stub.asked().len(), 1);
        settle(&mut layer, now + Duration::from_secs(121));
        assert_eq!(stub.asked().len(), 2);
    }

    #[test]
    fn numbers_are_grouped_in_thousands() {
        assert_eq!(grouped(999.0), "999");
        assert_eq!(grouped(12_825.4), "12,825");
        assert_eq!(grouped(1_234_567.0), "1,234,567");
        assert_eq!(grouped(-1_024.0), "-1,024");
    }
}
