//! The orbit Layer: every Tracked Object, as a Layer.

use std::sync::Arc;
use std::time::SystemTime;

use orion_core::catalogue::ObjectKind;
use orion_core::layer::{CardRow, FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::{Cache, Source};

use crate::feed::{Feed, Status};
use crate::layer::{Highlight, Layer, Toggle};
use crate::lines::{self, Segment};
use crate::objects::Instance;
use crate::tracked::{Details, Tracked};
use crate::ui::{format_latitude, format_longitude};

/// Linear colours with alpha.
const ORBIT_PATH_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
const GROUND_TRACK_COLOR: [f32; 4] = [1.0, 0.75, 0.15, 0.9];

pub struct OrbitLayer {
    tracked: Tracked,
    feed: Feed,
    on: bool,
    /// Whether the selected Tracked Object's Ground Track is drawn as well as its Orbit Path.
    show_ground_track: bool,
}

impl OrbitLayer {
    /// Starts from the freshest Element Sets available without the network, and
    /// refreshes them in the background if they are stale.
    pub fn start(
        source: Arc<dyn Source>,
        cache: Option<Cache>,
        now: SystemTime,
    ) -> anyhow::Result<Self> {
        let (feed, objects, records, groups) = Feed::start(source, cache, now)?;
        Ok(Self {
            tracked: Tracked::new(objects, records, groups),
            feed,
            on: true,
            show_ground_track: false,
        })
    }

    pub fn tracked(&self) -> &Tracked {
        &self.tracked
    }

    pub fn tracked_mut(&mut self) -> &mut Tracked {
        &mut self.tracked
    }

    pub fn set_show_ground_track(&mut self, show: bool) {
        self.show_ground_track = show;
    }

    /// Takes in freshly fetched Element Sets if a refresh has finished. Returns true
    /// if the Tracked Objects were replaced, which changes their indexes.
    pub fn poll(&mut self) -> bool {
        match self.feed.poll() {
            Some(refreshed) => {
                self.tracked
                    .replace(refreshed.objects, refreshed.records, refreshed.groups);
                true
            }
            None => false,
        }
    }

    pub fn feed_status(&self, now: SystemTime) -> Status {
        self.feed.status(now)
    }

    pub fn request_refresh(&mut self, now: SystemTime) {
        self.feed.request_refresh(now);
    }

    fn source_name(&self, now: SystemTime) -> &'static str {
        self.feed.status(now).source
    }
}

impl Layer for OrbitLayer {
    fn name(&self) -> &'static str {
        "Satellites"
    }

    fn theme(&self) -> Theme {
        Theme::Orbit
    }

    fn time_coverage(&self) -> TimeCoverage {
        // Positions are computed, so any moment the Simulation Clock allows can be shown.
        TimeCoverage::AnyMoment
    }

    fn is_on(&self) -> bool {
        self.on
    }

    fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    fn update(&mut self, at: JulianDate, highlight: Highlight) {
        self.tracked.propagate(at, highlight);
    }

    fn dots(&self) -> &[Instance] {
        self.tracked.instances()
    }

    fn lines(&self, selected: Option<usize>, at: JulianDate, out: &mut Vec<Segment>) {
        let Some(index) = selected else {
            return;
        };
        lines::push_polyline(out, &self.tracked.orbit_path(index, at), ORBIT_PATH_COLOR);
        if self.show_ground_track {
            lines::push_polyline(
                out,
                &self.tracked.ground_track(index, at),
                GROUND_TRACK_COLOR,
            );
        }
    }

    fn feature_name(&self, index: usize) -> Option<&str> {
        self.tracked
            .object(index)
            .map(|object| object.name.as_str())
    }

    fn card(&self, index: usize, at: JulianDate) -> Option<FeatureCard> {
        let details = self.tracked.details(index, at)?;
        Some(card(&details, self.source_name(SystemTime::now())))
    }

    fn stable_key(&self, index: usize) -> Option<u64> {
        self.tracked
            .object(index)
            .map(|object| u64::from(object.catalogue_number))
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        self.tracked.index_of(u32::try_from(key).ok()?)
    }

    fn toggles(&self) -> Vec<Toggle> {
        vec![Toggle {
            label: "Show Ground Track",
            on: self.show_ground_track,
        }]
    }

    fn set_toggles(&mut self, toggles: &[Toggle]) {
        if let Some(toggle) = toggles.first() {
            self.show_ground_track = toggle.on;
        }
    }
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Satellite => "Satellite",
        ObjectKind::RocketBody => "Rocket Body",
        ObjectKind::Debris => "Debris",
    }
}

/// Lays out what is known about a Tracked Object as a Feature card.
fn card(details: &Details, source: &str) -> FeatureCard {
    let mut identity = vec![
        CardRow::new("Catalogue number", details.catalogue_number.to_string()),
        CardRow::new("Designator", details.designator.clone()),
        CardRow::new("Kind", kind_name(details.kind)),
    ];
    match &details.record {
        Some(record) => {
            identity.push(CardRow::new("Owner", record.owner.clone()));
            identity.push(CardRow::new("Launched", record.launch_date.clone()));
            identity.push(CardRow::new("Launch site", record.launch_site.clone()));
        }
        None => identity.push(CardRow::new("Owner and launch", "Not yet on record")),
    }
    let mut sections = vec![identity];
    let mut notices = Vec::new();

    match details.now {
        Some(now) => sections.push(vec![
            CardRow::new("Latitude", format_latitude(now.over.latitude_deg)),
            CardRow::new("Longitude", format_longitude(now.over.longitude_deg)),
            CardRow::new("Altitude", format!("{:.0} km", now.over.altitude_km)),
            CardRow::new("Speed", format!("{:.2} km/s", now.speed_km_s)),
        ]),
        None => notices.push("No Position can be computed for this moment.".to_owned()),
    }

    sections.push(vec![
        CardRow::new("Period", format!("{:.1} min", details.period_minutes)),
        CardRow::new("Inclination", format!("{:.1}°", details.inclination_deg)),
        CardRow::new(
            "Highest point",
            format!("{:.0} km", details.apogee_altitude_km),
        ),
        CardRow::new(
            "Lowest point",
            format!("{:.0} km", details.perigee_altitude_km),
        ),
        CardRow::new("Eccentricity", format!("{:.4}", details.eccentricity)),
    ]);

    let epoch = details.epoch;
    sections.push(vec![
        CardRow::new(
            "Epoch",
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02} UTC",
                epoch.year, epoch.month, epoch.day, epoch.hour, epoch.minute
            ),
        ),
        CardRow::new(
            "Element Age",
            format!("{:.1} days", details.element_age_days),
        ),
    ]);
    if details.stale {
        notices.push("This Element Set is old. The Position may be far off.".to_owned());
    }

    FeatureCard {
        title: details.name.clone(),
        sections,
        notices,
        source: source.to_owned(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::feed::testing::Offline;

    /// The orbit Layer as the bundled Snapshot gives it, with nothing to fetch from.
    pub fn snapshot_layer() -> OrbitLayer {
        // With no cache there is nothing to refresh, so the time given does not matter.
        OrbitLayer::start(Arc::new(Offline), None, SystemTime::UNIX_EPOCH)
            .expect("a readable Snapshot")
    }

    fn moment() -> JulianDate {
        JulianDate::from_utc(2026, 11, 24, 21, 0, 0.0)
    }

    fn station(layer: &OrbitLayer) -> usize {
        layer
            .index_of(25_544)
            .expect("the space station is in the Snapshot")
    }

    #[test]
    fn a_tracked_object_keeps_its_catalogue_number_as_its_key() {
        let layer = snapshot_layer();
        let index = station(&layer);
        assert_eq!(layer.stable_key(index), Some(25_544));
        assert_eq!(layer.index_of(u64::MAX), None);
        assert!(layer.feature_name(index).unwrap().contains("ISS"));
        assert_eq!((layer.name(), layer.theme()), ("Satellites", Theme::Orbit));
    }

    #[test]
    fn the_card_lays_out_identity_position_orbit_and_element_set() {
        let layer = snapshot_layer();
        let card = layer.card(station(&layer), moment()).expect("a card");

        assert!(card.title.contains("ISS"));
        assert!(card.notices.is_empty());
        let labels: Vec<Vec<&str>> = card
            .sections
            .iter()
            .map(|section| section.iter().map(|row| row.label.as_str()).collect())
            .collect();
        assert_eq!(
            labels,
            [
                vec![
                    "Catalogue number",
                    "Designator",
                    "Kind",
                    "Owner",
                    "Launched",
                    "Launch site"
                ],
                vec!["Latitude", "Longitude", "Altitude", "Speed"],
                vec![
                    "Period",
                    "Inclination",
                    "Highest point",
                    "Lowest point",
                    "Eccentricity"
                ],
                vec!["Epoch", "Element Age"],
            ]
        );
        let value = |label: &str| {
            card.sections
                .iter()
                .flatten()
                .find(|row| row.label == label)
                .map(|row| row.value.clone())
                .unwrap()
        };
        assert_eq!(value("Kind"), "Satellite");
        assert_eq!(value("Launched"), "1998-11-20");
        assert_eq!(value("Inclination"), "51.6°");
    }

    #[test]
    fn an_old_element_set_puts_a_warning_on_the_card() {
        let layer = snapshot_layer();
        let later = JulianDate(moment().0 + 40.0);
        let card = layer.card(station(&layer), later).unwrap();
        assert_eq!(card.notices.len(), 1);
        assert!(card.notices[0].contains("old"));
    }

    #[test]
    fn the_selected_object_draws_its_orbit_path_and_optionally_its_ground_track() {
        let mut layer = snapshot_layer();
        let index = station(&layer);

        let mut segments = Vec::new();
        layer.lines(None, moment(), &mut segments);
        assert!(segments.is_empty());

        layer.lines(Some(index), moment(), &mut segments);
        let orbit_path_only = segments.len();
        assert!(orbit_path_only > 300);

        // The Ground Track is a switch offered for the selected object.
        let mut toggles = layer.toggles();
        assert_eq!(
            (toggles[0].label, toggles[0].on),
            ("Show Ground Track", false)
        );
        toggles[0].on = true;
        layer.set_toggles(&toggles);

        segments.clear();
        layer.lines(Some(index), moment(), &mut segments);
        assert_eq!(segments.len(), 2 * orbit_path_only);
    }
}
