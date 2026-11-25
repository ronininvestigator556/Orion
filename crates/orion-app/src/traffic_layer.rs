//! Layers of things that move: aircraft in the air around the middle of the view, as
//! heard by adsb.lol's volunteer receivers, and ships in the northern Baltic, as heard
//! by Finland's coastal receivers.
//!
//! Unlike a Layer of places, the Features move, and for flights the data depends on
//! where the view is. Each is fetched afresh every few minutes, and between fetches
//! each craft is moved along its last reported heading at its last reported speed.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

use glam::Vec3;
use orion_core::layer::{CardRow, FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::listing::{Fetcher, ListingError};
use orion_data::traffic::{
    AIRCRAFT_SOURCE, Aircraft, AircraftKind, MAX_RADIUS_NM, SHIP_DETAILS_URL, SHIP_POSITIONS_URL,
    SHIP_SOURCE, Ship, ShipKind, aircraft_url, parse_aircraft, parse_ships, snap,
};

use crate::feed::Refresh;
use crate::layer::{Highlight, Layer, LayerStatus, LayerView};
use crate::lines::Segment;
use crate::objects::{Instance, Marks};
use crate::ui::format_duration;

/// The least time between two fetches, however fast the view moves or the user asks.
const MIN_INTERVAL: Duration = Duration::from_secs(30);
/// How long after a failed fetch the next is tried.
const RETRY_AFTER: Duration = Duration::from_secs(120);
const EARTH_RADIUS_M: f64 = orion_core::EARTH_EQUATORIAL_RADIUS_KM * 1000.0;
const METRES_PER_FOOT: f64 = 0.3048;
const METRES_PER_NAUTICAL_MILE: f64 = 1852.0;

/// One of the kinds a traffic Layer divides its craft into: a name, a colour as
/// written for the screen, and whether it shows before the user touches the filter.
type Class = (&'static str, [u8; 3], bool);

/// One aircraft or ship as last heard, in the form every traffic Layer shares.
#[derive(Clone, Debug, PartialEq)]
pub struct Craft {
    /// Identifies it for as long as it is heard.
    pub key: u64,
    /// What it is called in lists and beside its dot.
    pub label: String,
    /// Everything the search looks through, in lower case.
    pub search: String,
    /// Which of its Layer's classes it belongs to.
    pub class: usize,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    /// Height above the sea, in metres.
    pub height_m: f64,
    /// Speed over the ground in knots.
    pub speed_kt: Option<f64>,
    /// Direction of travel over the ground, in degrees clockwise from north.
    pub track_deg: Option<f64>,
    /// True if it is parked, moored or otherwise not going anywhere: it is drawn
    /// where it was heard and trails no line.
    pub at_rest: bool,
    /// How long before the answer was made its position was heard, in seconds.
    pub position_age_s: f64,
    pub rows: Vec<CardRow>,
    /// Things a reader should be warned about, such as a declared emergency.
    pub notices: Vec<String>,
    /// How large its dot is, as a multiple of the usual size.
    pub dot_scale: f32,
}

type Fetch = fn(&dyn Fetcher, (i32, i32)) -> Result<Vec<Craft>, ListingError>;

/// Everything that makes one traffic Layer different from another.
pub struct TrafficSpec {
    pub name: &'static str,
    pub source: &'static str,
    pub classes: &'static [Class],
    /// Credit that stays on screen while the Layer is on.
    pub attribution: &'static str,
    /// What a reader should know about the Layer, shown after its status.
    pub note: &'static str,
    /// How long the craft on hand stay fresh while the view stays put.
    pub refresh_every: Duration,
    /// Whether craft are fetched for the place the view is over, to the nearest
    /// degree. If not, they are fetched for the Layer's whole area at once.
    pub follows_view: bool,
    /// The longest a craft is moved along its last heading before it is left where
    /// that puts it, in seconds. Past this the estimate is worth little.
    pub max_estimate_seconds: f64,
    /// How many seconds of travel the line behind each craft shows.
    pub tail_seconds: f64,
    /// Fetches the craft around a whole-degree place.
    pub fetch: Fetch,
}

fn linear(color: [u8; 3]) -> [u8; 3] {
    color.map(|channel| ((f32::from(channel) / 255.0).powf(2.2) * 255.0).round() as u8)
}

/// A number with a comma between thousands, such as `12,825`.
fn grouped(number: f64) -> String {
    let digits = format!("{:.0}", number.abs());
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    if number < 0.0 && text != "0" {
        text.insert(0, '-');
    }
    text
}

fn rows(pairs: Vec<(&str, String)>) -> Vec<CardRow> {
    pairs
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(label, value)| CardRow::new(label, value))
        .collect()
}

const AIRCRAFT_CLASSES: [Class; 5] = [
    ("Airliners and large aircraft", [198, 255, 0], true),
    ("Light aircraft", [174, 234, 0], true),
    ("Helicopters", [118, 255, 3], true),
    ("Other and unknown", [178, 255, 89], true),
    ("On the ground", [158, 158, 158], false),
];

fn aircraft_craft(aircraft: Aircraft) -> Craft {
    let class = match (aircraft.on_ground, aircraft.kind) {
        (true, _) => 4,
        (false, AircraftKind::Large) => 0,
        (false, AircraftKind::Light) => 1,
        (false, AircraftKind::Helicopter) => 2,
        (false, AircraftKind::Other) => 3,
    };
    let name = [
        &aircraft.callsign,
        &aircraft.registration,
        &aircraft.address,
    ]
    .into_iter()
    .find(|text| !text.is_empty())
    .cloned()
    .unwrap_or_default();
    let label = if aircraft.type_code.is_empty() {
        name
    } else {
        format!("{name} ({})", aircraft.type_code)
    };
    let altitude = match (aircraft.on_ground, aircraft.altitude_ft) {
        (true, _) => "On the ground".to_owned(),
        (false, Some(feet)) => format!(
            "{} ft ({} m)",
            grouped(feet),
            grouped(feet * METRES_PER_FOOT)
        ),
        (false, None) => String::new(),
    };
    let (climbing, descending) = match aircraft.climb_fpm {
        Some(rate) if rate >= 100.0 => (format!("{} ft/min", grouped(rate)), String::new()),
        Some(rate) if rate <= -100.0 => (String::new(), format!("{} ft/min", grouped(-rate))),
        _ => (String::new(), String::new()),
    };
    Craft {
        key: aircraft.key,
        search: format!(
            "{} {} {} {}",
            aircraft.callsign, aircraft.registration, aircraft.type_code, aircraft.address
        )
        .to_lowercase(),
        class,
        latitude_deg: aircraft.latitude_deg,
        longitude_deg: aircraft.longitude_deg,
        height_m: aircraft.altitude_ft.unwrap_or(0.0).max(0.0) * METRES_PER_FOOT,
        speed_kt: aircraft.speed_kt,
        track_deg: aircraft.track_deg,
        at_rest: aircraft.on_ground,
        position_age_s: aircraft.position_age_s,
        rows: rows(vec![
            ("Callsign", aircraft.callsign.clone()),
            ("Registration", aircraft.registration.clone()),
            ("Type", aircraft.type_code.clone()),
            ("Kind", AIRCRAFT_CLASSES[class].0.to_owned()),
            ("Altitude", altitude),
            (
                "Ground speed",
                aircraft.speed_kt.map_or_else(String::new, |knots| {
                    format!("{knots:.0} kt ({:.0} km/h)", knots * 1.852)
                }),
            ),
            (
                "Heading",
                aircraft
                    .track_deg
                    .map_or_else(String::new, |track| format!("{track:.0}°")),
            ),
            ("Climbing", climbing),
            ("Descending", descending),
            ("Squawk", aircraft.squawk.clone()),
            ("Transponder address", aircraft.address.clone()),
        ]),
        notices: aircraft
            .emergency
            .iter()
            .map(|declared| format!("Declaring an emergency: {declared}."))
            .collect(),
        dot_scale: if aircraft.kind == AircraftKind::Large {
            1.8
        } else {
            1.3
        },
        label,
    }
}

fn fetch_aircraft(fetcher: &dyn Fetcher, region: (i32, i32)) -> Result<Vec<Craft>, ListingError> {
    let url = aircraft_url(f64::from(region.0), f64::from(region.1), MAX_RADIUS_NM);
    let text = fetcher.get(&url, None).map_err(ListingError::Fetch)?;
    let aircraft = parse_aircraft(&text).map_err(ListingError::Parse)?;
    Ok(aircraft.into_iter().map(aircraft_craft).collect())
}

/// Aircraft in the air around the middle of the view.
pub static FLIGHTS: TrafficSpec = TrafficSpec {
    name: "Flights",
    source: AIRCRAFT_SOURCE,
    classes: &AIRCRAFT_CLASSES,
    attribution: "Flights: adsb.lol contributors",
    note: "Aircraft within about 460 km of the middle of the view, which is all the \
           service is told about, to the nearest degree. Between refreshes each is moved \
           along its last heading.",
    refresh_every: Duration::from_secs(180),
    follows_view: true,
    max_estimate_seconds: 300.0,
    tail_seconds: 60.0,
    fetch: fetch_aircraft,
};

const SHIP_CLASSES: [Class; 6] = [
    ("Cargo ships", [128, 222, 234], true),
    ("Tankers", [255, 138, 128], true),
    ("Passenger ships", [255, 255, 255], true),
    ("Tugs and working craft", [255, 224, 130], true),
    ("Fishing boats and yachts", [165, 214, 167], true),
    ("Other and unknown", [176, 190, 197], true),
];

fn ship_craft(ship: Ship) -> Craft {
    let class = match ship.kind {
        ShipKind::Cargo => 0,
        ShipKind::Tanker => 1,
        ShipKind::Passenger => 2,
        ShipKind::Service => 3,
        ShipKind::FishingOrLeisure => 4,
        ShipKind::Other => 5,
    };
    let label = if ship.name.is_empty() {
        format!("Ship {}", ship.mmsi)
    } else {
        ship.name.clone()
    };
    Craft {
        key: ship.key,
        search: format!("{} {} {}", ship.name, ship.call_sign, ship.mmsi).to_lowercase(),
        class,
        latitude_deg: ship.latitude_deg,
        longitude_deg: ship.longitude_deg,
        height_m: 0.0,
        speed_kt: ship.speed_kt,
        track_deg: ship.course_deg,
        at_rest: ship.at_rest || ship.speed_kt.is_some_and(|knots| knots < 0.3),
        position_age_s: ship.position_age_s,
        rows: rows(vec![
            ("Name", ship.name.clone()),
            ("Kind", SHIP_CLASSES[class].0.to_owned()),
            ("Doing", ship.status.to_owned()),
            (
                "Speed",
                ship.speed_kt.map_or_else(String::new, |knots| {
                    format!("{knots:.1} kt ({:.0} km/h)", knots * 1.852)
                }),
            ),
            (
                "Course",
                ship.course_deg
                    .map_or_else(String::new, |course| format!("{course:.0}°")),
            ),
            ("Destination, as entered aboard", ship.destination.clone()),
            (
                "Draught",
                ship.draught_m
                    .map_or_else(String::new, |metres| format!("{metres:.1} m")),
            ),
            ("Call sign", ship.call_sign.clone()),
            (
                "IMO number",
                ship.imo.map_or_else(String::new, |imo| imo.to_string()),
            ),
            ("Radio identity (MMSI)", ship.mmsi.to_string()),
        ]),
        notices: Vec::new(),
        dot_scale: 1.5,
        label,
    }
}

fn fetch_ships(fetcher: &dyn Fetcher, _region: (i32, i32)) -> Result<Vec<Craft>, ListingError> {
    let positions = fetcher
        .get(SHIP_POSITIONS_URL, None)
        .map_err(ListingError::Fetch)?;
    let details = fetcher
        .get(SHIP_DETAILS_URL, None)
        .map_err(ListingError::Fetch)?;
    let ships = parse_ships(&positions, &details).map_err(ListingError::Parse)?;
    Ok(ships.into_iter().map(ship_craft).collect())
}

/// Ships in the northern Baltic. The one sea whose authority publishes its ships'
/// positions for anyone to use without a key.
pub static SHIPS: TrafficSpec = TrafficSpec {
    name: "Ships: northern Baltic",
    source: SHIP_SOURCE,
    classes: &SHIP_CLASSES,
    attribution: "Ships: Fintraffic / digitraffic.fi, CC BY 4.0",
    note: "Ships heard by Finland's coastal receivers in the past half hour: the Gulf of \
           Finland, the Gulf of Bothnia and the sea between. Ships elsewhere need a key \
           of your own from a ship-tracking service, which Orion cannot use yet.",
    refresh_every: Duration::from_secs(180),
    follows_view: false,
    max_estimate_seconds: 600.0,
    tail_seconds: 600.0,
    fetch: fetch_ships,
};

/// Where a craft is `seconds` after its position was heard, in Earth radii in the
/// Earth-fixed frame: moved along its heading at its speed, and at its height.
fn position_after(craft: &Craft, seconds: f64, limit: f64) -> Vec3 {
    let (mut latitude, mut longitude) = (
        craft.latitude_deg.to_radians(),
        craft.longitude_deg.to_radians(),
    );
    if let (false, Some(speed), Some(track)) = (craft.at_rest, craft.speed_kt, craft.track_deg) {
        let seconds = seconds.clamp(-limit, limit);
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
    let radius = 1.0 + craft.height_m / EARTH_RADIUS_M;
    Vec3::new(
        (radius * latitude.cos() * longitude.cos()) as f32,
        (radius * latitude.cos() * longitude.sin()) as f32,
        (radius * latitude.sin()) as f32,
    )
}

type Fetched = Result<Vec<Craft>, ListingError>;

pub struct TrafficLayer {
    spec: &'static TrafficSpec,
    fetcher: Arc<dyn Fetcher>,
    on: bool,
    craft: Vec<Craft>,
    class_on: Vec<bool>,
    search: String,
    listed: Vec<bool>,
    listed_count: usize,
    /// Where each craft is estimated to be now.
    positions: Vec<Vec3>,
    dots: Vec<Instance>,
    /// The whole-degree place the view is over.
    view: Option<(i32, i32)>,
    /// The whole-degree place the craft on hand were fetched around.
    region: Option<(i32, i32)>,
    fetched_at: Option<SystemTime>,
    in_flight: Option<(Receiver<Fetched>, (i32, i32))>,
    last_attempt: Option<SystemTime>,
    failure: Option<String>,
}

impl TrafficLayer {
    /// A Layer that is off and has fetched nothing.
    pub fn new(spec: &'static TrafficSpec, fetcher: Arc<dyn Fetcher>) -> Self {
        Self {
            spec,
            fetcher,
            on: false,
            craft: Vec::new(),
            class_on: spec.classes.iter().map(|(_, _, on)| *on).collect(),
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

    fn set_craft(&mut self, craft: Vec<Craft>, region: (i32, i32), now: SystemTime) {
        self.craft = craft;
        self.region = Some(region);
        self.fetched_at = Some(now);
        self.refilter();
    }

    fn refilter(&mut self) {
        self.listed = self
            .craft
            .iter()
            .map(|craft| {
                self.class_on.get(craft.class).copied().unwrap_or(true)
                    && craft.search.contains(&self.search)
            })
            .collect();
        self.listed_count = self.listed.iter().filter(|listed| **listed).count();
    }

    /// The place craft should be fetched for: where the view is, or the one place
    /// that stands for a Layer's whole area.
    fn wanted_region(&self) -> Option<(i32, i32)> {
        if self.spec.follows_view {
            self.view
        } else {
            Some((0, 0))
        }
    }

    /// Takes in a fetch that has finished, and starts one if the craft on hand are
    /// stale or were fetched for somewhere the view has left.
    fn tick(&mut self, now: SystemTime) {
        if !self.on {
            return;
        }
        if let Some((receiver, region)) = &self.in_flight {
            let region = *region;
            match receiver.try_recv() {
                Ok(Ok(craft)) => {
                    self.in_flight = None;
                    self.failure = None;
                    self.set_craft(craft, region, now);
                }
                Ok(Err(error)) => {
                    self.in_flight = None;
                    log::warn!("fetching {}: {error}", self.spec.name);
                    self.failure = Some(error.to_string());
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.in_flight = None;
                    self.failure = Some("the fetch stopped unexpectedly".to_owned());
                }
            }
        }
        let Some(wanted) = self.wanted_region() else {
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
        let moved_on = self.region != Some(wanted);
        let stale = since(self.fetched_at).is_none_or(|age| age >= self.spec.refresh_every);
        if may_try && (moved_on || stale) {
            self.start_fetch(wanted, now);
        }
    }

    fn start_fetch(&mut self, region: (i32, i32), now: SystemTime) {
        if self.in_flight.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let fetcher = Arc::clone(&self.fetcher);
        let fetch = self.spec.fetch;
        std::thread::spawn(move || {
            // The receiver is gone only if the app is shutting down.
            let _ = sender.send(fetch(&*fetcher, region));
        });
        self.in_flight = Some((receiver, region));
        self.last_attempt = Some(now);
    }

    /// How long ago the craft on hand were fetched, in seconds.
    fn seconds_since_fetch(&self, now: SystemTime) -> f64 {
        self.fetched_at
            .map(|at| now.duration_since(at).unwrap_or_default().as_secs_f64())
            .unwrap_or(0.0)
    }

    fn rebuild(&mut self, now: SystemTime, highlight: Highlight) {
        let elapsed = self.seconds_since_fetch(now);
        let limit = self.spec.max_estimate_seconds;
        self.positions = self
            .craft
            .iter()
            .map(|craft| position_after(craft, elapsed + craft.position_age_s, limit))
            .collect();
        let colors: Vec<[u8; 3]> = self
            .spec
            .classes
            .iter()
            .map(|(_, color, _)| linear(*color))
            .collect();
        self.dots = self
            .craft
            .iter()
            .enumerate()
            .map(|(index, craft)| {
                if !self.listed[index] {
                    return Instance::HIDDEN;
                }
                let [r, g, b] = colors.get(craft.class).copied().unwrap_or([255; 3]);
                let marks = Marks {
                    selected: highlight.selected == Some(index),
                    hovered: highlight.hovered == Some(index),
                    stale: false,
                };
                Instance::new(self.positions[index], [r, g, b, 255], marks).scaled(craft.dot_scale)
            })
            .collect();
    }
}

impl Layer for TrafficLayer {
    fn name(&self) -> &'static str {
        self.spec.name
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
        let [r, g, b] = self
            .spec
            .classes
            .first()
            .map_or([255; 3], |(_, color, _)| linear(*color));
        [r, g, b, 255]
    }

    fn counts(&self) -> (usize, usize) {
        (self.listed_count, self.craft.len())
    }

    fn status(&self, now: SystemTime) -> LayerStatus {
        let source = self.spec.source;
        let age = self
            .fetched_at
            .map(|at| format_duration(now.duration_since(at).unwrap_or_default()));
        let mut text = match (&self.failure, &age) {
            (Some(reason), Some(age)) => format!(
                "Could not reach {source}: {reason}. Showing what was heard {age} ago; trying again shortly."
            ),
            (Some(reason), None) => {
                format!("Could not reach {source}: {reason}. Trying again shortly.")
            }
            (None, Some(age)) => format!("From {source}, heard {age} ago."),
            (None, None) => format!("Fetching from {source}."),
        };
        text.push(' ');
        text.push_str(self.spec.note);
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
        if let (true, false, Some(region)) = (self.on, recently, self.wanted_region()) {
            self.failure = None;
            self.start_fetch(region, now);
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
        self.craft
            .iter()
            .enumerate()
            .filter(|(index, _)| self.listed[*index])
            .take(limit)
            .map(|(index, craft)| (index, craft.label.clone()))
            .collect()
    }

    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        for ((label, [r, g, b], _), on) in self.spec.classes.iter().zip(&mut self.class_on) {
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

    /// A short line behind each craft under way, showing where it has come from.
    fn lines(&self, _selected: Option<usize>, _at: JulianDate, out: &mut Vec<Segment>) {
        let elapsed = self.seconds_since_fetch(SystemTime::now());
        let limit = self.spec.max_estimate_seconds;
        for (index, craft) in self.craft.iter().enumerate() {
            if !self.listed[index] || craft.at_rest || craft.speed_kt.is_none() {
                continue;
            }
            let Some(head) = self.positions.get(index) else {
                continue;
            };
            let since_heard = (elapsed + craft.position_age_s).min(limit);
            let tail = position_after(craft, since_heard - self.spec.tail_seconds, limit);
            let [r, g, b] = self
                .spec
                .classes
                .get(craft.class)
                .map_or([255; 3], |(_, color, _)| linear(*color))
                .map(|channel| f32::from(channel) / 255.0);
            out.push(Segment::new(tail, *head, [r, g, b, 0.45]));
        }
    }

    fn feature_name(&self, index: usize) -> Option<&str> {
        self.craft.get(index).map(|craft| craft.label.as_str())
    }

    fn card(&self, index: usize, _at: JulianDate) -> Option<FeatureCard> {
        let craft = self.craft.get(index)?;
        let heard = self.seconds_since_fetch(SystemTime::now()) + craft.position_age_s;
        let estimated = if craft.at_rest {
            String::new()
        } else {
            " and estimated since from its speed and heading".to_owned()
        };
        Some(FeatureCard {
            title: craft.label.clone(),
            sections: vec![craft.rows.clone()],
            details: vec![format!(
                "Position last heard {} ago{estimated}.",
                if heard < 90.0 {
                    format!("{heard:.0} seconds")
                } else {
                    format_duration(Duration::from_secs_f64(heard))
                }
            )],
            notices: craft.notices.clone(),
            source: self.spec.source.to_owned(),
        })
    }

    fn stable_key(&self, index: usize) -> Option<u64> {
        self.craft.get(index).map(|craft| craft.key)
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        self.craft.iter().position(|craft| craft.key == key)
    }

    fn is_loading(&self) -> bool {
        self.on && self.fetched_at.is_none() && self.failure.is_none()
    }

    fn attribution(&self) -> Option<&'static str> {
        Some(self.spec.attribution)
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

    /// Answers every fetch according to what was asked for and remembers the askings.
    struct Stub {
        answer: fn(&str) -> Result<String, FetchError>,
        asked: Mutex<Vec<String>>,
    }

    impl Stub {
        fn answering(answer: fn(&str) -> Result<String, FetchError>) -> Arc<Self> {
            Arc::new(Self {
                answer,
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
            (self.answer)(url)
        }
    }

    const AIRCRAFT: &str = r#"{"ac":[
        {"hex":"4caa41","flight":"EIN142  ","r":"EI-EIN","t":"A333","alt_baro":35000,"gs":480.0,
         "track":90.0,"baro_rate":-1024,"squawk":"3607","emergency":"none","category":"A5",
         "lat":0.0,"lon":10.0,"seen_pos":0.0},
        {"hex":"406abc","flight":"GABCD   ","r":"G-ABCD","t":"C172","alt_baro":"ground","gs":4.0,
         "track":120.0,"category":"A1","lat":51.47,"lon":-0.46,"seen_pos":2.0},
        {"hex":"43c001","alt_baro":1500,"gs":90,"track":10,"category":"A7","emergency":"general",
         "lat":51.0,"lon":0.1,"seen_pos":0.0}]}"#;

    fn aircraft(_url: &str) -> Result<String, FetchError> {
        Ok(AIRCRAFT.to_owned())
    }

    fn looking_at(layer: &mut TrafficLayer, latitude_deg: f64, longitude_deg: f64) {
        layer.set_view(&LayerView {
            latitude_deg,
            longitude_deg,
            ..LayerView::default()
        });
    }

    fn settle(layer: &mut TrafficLayer, now: SystemTime) {
        for _ in 0..500 {
            layer.tick(now);
            if layer.in_flight.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the fetch never finished");
    }

    fn flights(stub: &Arc<Stub>) -> (TrafficLayer, SystemTime) {
        let mut layer = TrafficLayer::new(&FLIGHTS, Arc::clone(stub) as Arc<dyn Fetcher>);
        let now = SystemTime::now();
        looking_at(&mut layer, 51.47, -0.46);
        layer.set_on(true);
        settle(&mut layer, now);
        (layer, now)
    }

    #[test]
    fn nothing_is_asked_until_the_layer_is_on_and_then_only_roughly_where() {
        let stub = Stub::answering(aircraft);
        let mut layer = TrafficLayer::new(&FLIGHTS, Arc::clone(&stub) as Arc<dyn Fetcher>);
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
        let stub = Stub::answering(aircraft);
        let (mut layer, now) = flights(&stub);

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
    fn between_fetches_craft_move_along_their_heading() {
        let stub = Stub::answering(aircraft);
        let (mut layer, now) = flights(&stub);

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
        let stub = Stub::answering(aircraft);
        let (mut layer, now) = flights(&stub);
        layer.class_on[4] = true;
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
        assert_eq!(value("Climbing"), None);
        assert_eq!(card.source, "adsb.lol");
        assert!(card.notices.is_empty());
        assert!(
            card.details[0].contains("estimated since"),
            "{}",
            card.details[0]
        );

        let parked = layer.card(1, JulianDate::J2000).unwrap();
        assert_eq!(
            parked.sections[0][4],
            CardRow::new("Altitude", "On the ground")
        );
        assert!(!parked.details[0].contains("estimated"));

        // No callsign or registration: the transponder's address names it.
        let helicopter = layer.card(2, JulianDate::J2000).unwrap();
        assert_eq!(helicopter.title, "43c001");
        assert_eq!(helicopter.notices, ["Declaring an emergency: general."]);
    }

    #[test]
    fn the_search_finds_aircraft_by_callsign_registration_or_type() {
        let stub = Stub::answering(aircraft);
        let (mut layer, _) = flights(&stub);
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
        let stub = Stub::answering(|_| Err(FetchError::Refused(429)));
        let (mut layer, now) = flights(&stub);
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

    fn ships(url: &str) -> Result<String, FetchError> {
        Ok(if url.ends_with("/locations") {
            r#"{"features":[
                {"mmsi":255802540,"geometry":{"coordinates":[24.95,60.10]},
                 "properties":{"sog":12.0,"cog":180.0,"navStat":0,"timestampExternal":1795605524000}},
                {"mmsi":230000001,"geometry":{"coordinates":[21.2148,62.3527]},
                 "properties":{"sog":0.0,"cog":360.0,"navStat":5,"timestampExternal":1795605464000}}]}"#
        } else {
            r#"[{"callSign":"CQPB","destination":"NLRTM","draught":59,"imo":9350783,
                 "mmsi":255802540,"name":"ALAND","shipType":70}]"#
        }
        .to_owned())
    }

    #[test]
    fn ships_are_fetched_whole_wherever_the_view_is() {
        let stub = Stub::answering(ships);
        let mut layer = TrafficLayer::new(&SHIPS, Arc::clone(&stub) as Arc<dyn Fetcher>);
        let now = SystemTime::now();
        layer.set_on(true);
        // No view has been set, and none is needed.
        settle(&mut layer, now);
        assert_eq!(stub.asked(), [SHIP_POSITIONS_URL, SHIP_DETAILS_URL]);
        assert_eq!(layer.counts(), (2, 2));

        // Moving the view asks for nothing more.
        looking_at(&mut layer, -30.0, 150.0);
        layer.tick(now + Duration::from_secs(100));
        assert_eq!(stub.asked().len(), 2);
        settle(&mut layer, now + Duration::from_secs(181));
        assert_eq!(stub.asked().len(), 4);
    }

    #[test]
    fn a_ship_shows_what_it_says_of_itself_and_moves_slowly() {
        let stub = Stub::answering(ships);
        let mut layer = TrafficLayer::new(&SHIPS, Arc::clone(&stub) as Arc<dyn Fetcher>);
        let now = SystemTime::now();
        layer.set_on(true);
        settle(&mut layer, now);

        let card = layer.card(0, JulianDate::J2000).unwrap();
        assert_eq!(card.title, "ALAND");
        let value = |label: &str| {
            card.sections[0]
                .iter()
                .find(|row| row.label == label)
                .map(|row| row.value.as_str())
        };
        assert_eq!(value("Kind"), Some("Cargo ships"));
        assert_eq!(value("Doing"), Some("Under way using its engine"));
        assert_eq!(value("Speed"), Some("12.0 kt (22 km/h)"));
        assert_eq!(value("Destination, as entered aboard"), Some("NLRTM"));
        assert_eq!(value("Draught"), Some("5.9 m"));
        assert_eq!(value("IMO number"), Some("9350783"));
        assert_eq!(card.source, "Fintraffic");

        // The moored one is known only by its number, and trails no line.
        assert_eq!(layer.feature_name(1), Some("Ship 230000001"));
        layer.rebuild(now, Highlight::default());
        let mut lines = Vec::new();
        layer.lines(None, JulianDate::J2000, &mut lines);
        assert_eq!(lines.len(), 1);

        // Ten minutes at 12 knots due south is 2 nautical miles: a thirtieth of a degree.
        layer.rebuild(now + Duration::from_secs(600), Highlight::default());
        let position = layer.position(0).unwrap();
        let latitude = position.z.asin().to_degrees();
        assert!(
            (latitude - (60.10 - 2.0 / 60.0)).abs() < 0.001,
            "{latitude}"
        );
        layer.set_search("aland");
        assert_eq!(layer.counts(), (1, 2));
    }

    #[test]
    fn numbers_are_grouped_in_thousands() {
        assert_eq!(grouped(999.0), "999");
        assert_eq!(grouped(12_825.4), "12,825");
        assert_eq!(grouped(1_234_567.0), "1,234,567");
        assert_eq!(grouped(-1_024.0), "-1,024");
    }
}
