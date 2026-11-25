//! The Tracked Objects on show and their Positions for the current frame.

use std::time::{Duration, Instant};

use glam::{DVec3, Vec3};
use orion_core::EARTH_EQUATORIAL_RADIUS_KM;
use orion_core::catalogue::{CatalogueRecords, ObjectKind, TrackedObject};
use orion_core::codes::{launch_site_name, owner_name};
use orion_core::filter::{Filter, GroupLists};
use orion_core::frames::{Geodetic, geodetic};
use orion_core::time::{CalendarTime, JulianDate};
use rayon::prelude::*;

use crate::objects::{Instance, Marks};

/// Element Sets older than this give Positions too unreliable to show at full strength.
const STALE_AFTER_DAYS: f64 = 30.0;
/// Points used to draw one revolution of an Orbit Path or Ground Track.
const PATH_SAMPLES: usize = 361;

/// Which Tracked Objects, by index, should stand out in the next frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Highlight {
    pub selected: Option<usize>,
    pub hovered: Option<usize>,
}

/// Everything the information panel shows about one Tracked Object.
#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub name: String,
    pub catalogue_number: u32,
    pub designator: String,
    pub kind: ObjectKind,
    pub epoch: CalendarTime,
    pub element_age_days: f64,
    /// The Element Set is too old for the Position to be trusted.
    pub stale: bool,
    pub period_minutes: f64,
    pub inclination_deg: f64,
    pub eccentricity: f64,
    pub apogee_altitude_km: f64,
    pub perigee_altitude_km: f64,
    /// `None` if the object has no Position at this moment.
    pub now: Option<Now>,
    /// `None` if the object has no Catalogue Record on hand, as can happen for a new launch.
    pub record: Option<RecordDetails>,
}

/// A Catalogue Record with its codes spelled out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordDetails {
    pub owner: String,
    /// Launch date as `YYYY-MM-DD`.
    pub launch_date: String,
    pub launch_site: String,
}

/// Spells out a code where its name is known, such as `United States (US)`.
fn spell_out(code: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{name} ({code})"),
        None => code.to_owned(),
    }
}

/// Where a Tracked Object is at the moment on show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Now {
    pub over: Geodetic,
    pub speed_km_s: f64,
}

/// The colour of a Tracked Object's dot, as linear colour with alpha from 0 to 255.
pub fn kind_color(kind: ObjectKind) -> [u8; 4] {
    match kind {
        ObjectKind::Satellite => [89, 191, 255, 255],
        ObjectKind::RocketBody => [255, 128, 31, 255],
        ObjectKind::Debris => [191, 51, 51, 191],
    }
}

fn to_earth_radii(position_km: [f64; 3]) -> Vec3 {
    (DVec3::from_array(position_km) / EARTH_EQUATORIAL_RADIUS_KM).as_vec3()
}

pub struct Tracked {
    objects: Vec<TrackedObject>,
    records: CatalogueRecords,
    groups: GroupLists,
    filter: Filter,
    /// Whether each Tracked Object passes the filter, in the same order as `objects`.
    visible: Vec<bool>,
    /// Indexes of the objects that pass the filter, in name order.
    matches: Vec<usize>,
    /// One per Tracked Object, in the same order, refreshed by `propagate`.
    instances: Vec<Instance>,
    /// The Epoch that half the Element Sets are older than; `None` if there are none.
    median_epoch: Option<JulianDate>,
    last_propagation: Duration,
}

/// How many Tracked Objects are on show, for the diagnostics panel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub satellites: usize,
    pub rocket_bodies: usize,
    pub debris: usize,
    /// Objects with no Position at the current moment.
    pub without_position: usize,
    /// Objects that pass the filter.
    pub shown: usize,
    /// The Element Age, in days, that half the Element Sets exceed.
    pub median_element_age_days: Option<f64>,
    pub propagation_ms: f32,
}

impl Tracked {
    pub fn new(objects: Vec<TrackedObject>, records: CatalogueRecords, groups: GroupLists) -> Self {
        // The median, not the newest: a few Element Sets are published with an Epoch
        // days in the future, which would make the whole set look fresher than it is.
        let mut epochs: Vec<f64> = objects
            .iter()
            .map(|object| object.element_set.epoch().0)
            .collect();
        epochs.sort_by(f64::total_cmp);
        let median_epoch = epochs.get(epochs.len() / 2).copied().map(JulianDate);

        let mut tracked = Self {
            objects,
            records,
            groups,
            filter: Filter::default(),
            visible: Vec::new(),
            matches: Vec::new(),
            instances: Vec::new(),
            median_epoch,
            last_propagation: Duration::ZERO,
        };
        tracked.apply_filter();
        tracked
    }

    /// Which Tracked Objects are on show.
    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Changes which Tracked Objects are on show.
    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
        self.apply_filter();
    }

    /// Works out afresh which objects pass the filter.
    fn apply_filter(&mut self) {
        self.visible = self
            .objects
            .par_iter()
            .map(|object| self.filter.matches(object, &self.groups))
            .collect();
        self.matches = (0..self.objects.len())
            .filter(|index| self.visible[*index])
            .collect();
        self.matches
            .sort_by(|a, b| self.objects[*a].name.cmp(&self.objects[*b].name));
    }

    /// Indexes of the Tracked Objects that pass the filter, in name order.
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    /// How many Tracked Objects there are, shown or not.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Swaps in a new set of Tracked Objects after a refresh, with new Catalogue Records
    /// and Group lists if the refresh brought them. The filter stays as it was.
    pub fn replace(
        &mut self,
        objects: Vec<TrackedObject>,
        records: Option<CatalogueRecords>,
        groups: Option<GroupLists>,
    ) {
        let records = records.unwrap_or_else(|| std::mem::take(&mut self.records));
        let groups = groups.unwrap_or_else(|| std::mem::take(&mut self.groups));
        let filter = self.filter.clone();
        *self = Self::new(objects, records, groups);
        self.set_filter(filter);
    }

    /// Computes every Tracked Object's Position at the given moment.
    pub fn propagate(&mut self, at: JulianDate, highlight: Highlight) {
        let started = Instant::now();
        let visible = &self.visible;
        self.objects
            .par_iter()
            .enumerate()
            .map(|(index, object)| {
                // The selected object stays on show even when the filter excludes it,
                // so its panel never describes something invisible.
                if !visible[index] && highlight.selected != Some(index) {
                    return Instance::HIDDEN;
                }
                let Some(position_km) = object.element_set.position(at) else {
                    return Instance::HIDDEN;
                };
                let marks = Marks {
                    selected: highlight.selected == Some(index),
                    hovered: highlight.hovered == Some(index),
                    stale: object.element_set.age_days(at) > STALE_AFTER_DAYS,
                };
                Instance::new(to_earth_radii(position_km), kind_color(object.kind), marks)
            })
            .collect_into_vec(&mut self.instances);
        self.last_propagation = started.elapsed();
    }

    /// The index of the Tracked Object with this catalogue number, if it is on show.
    pub fn index_of(&self, catalogue_number: u32) -> Option<usize> {
        self.objects
            .iter()
            .position(|object| object.catalogue_number == catalogue_number)
    }

    pub fn object(&self, index: usize) -> Option<&TrackedObject> {
        self.objects.get(index)
    }

    /// What the information panel shows about one Tracked Object at the given moment.
    pub fn details(&self, index: usize, at: JulianDate) -> Option<Details> {
        let object = self.objects.get(index)?;
        let element_set = &object.element_set;
        let element_age_days = element_set.age_days(at);
        Some(Details {
            name: object.name.clone(),
            catalogue_number: object.catalogue_number,
            designator: object.designator.clone(),
            kind: object.kind,
            epoch: element_set.epoch().to_utc(),
            element_age_days,
            stale: element_age_days > STALE_AFTER_DAYS,
            period_minutes: element_set.period_minutes(),
            inclination_deg: element_set.inclination_deg(),
            eccentricity: element_set.eccentricity(),
            apogee_altitude_km: element_set.apogee_altitude_km(),
            perigee_altitude_km: element_set.perigee_altitude_km(),
            now: element_set.state(at).map(|state| Now {
                over: geodetic(state.position_km),
                speed_km_s: state.speed_km_s,
            }),
            record: self
                .records
                .get(&object.catalogue_number)
                .map(|record| RecordDetails {
                    owner: spell_out(&record.owner, owner_name(&record.owner)),
                    launch_date: record.launch_date.clone(),
                    launch_site: spell_out(
                        &record.launch_site,
                        launch_site_name(&record.launch_site),
                    ),
                }),
        })
    }

    /// The Orbit Path of one Tracked Object from the given moment, in Earth radii.
    pub fn orbit_path(&self, index: usize, at: JulianDate) -> Vec<Vec3> {
        self.objects.get(index).map_or_else(Vec::new, |object| {
            object
                .element_set
                .orbit_path(at, PATH_SAMPLES)
                .into_iter()
                .map(to_earth_radii)
                .collect()
        })
    }

    /// The Ground Track of one Tracked Object for one revolution from the given moment,
    /// as points on the globe's surface, in Earth radii.
    pub fn ground_track(&self, index: usize, at: JulianDate) -> Vec<Vec3> {
        self.objects.get(index).map_or_else(Vec::new, |object| {
            object
                .element_set
                .ground_path(at, PATH_SAMPLES)
                .into_iter()
                // The globe is drawn as a sphere, so "beneath" is towards its centre.
                .map(|position_km| to_earth_radii(position_km).normalize())
                .collect()
        })
    }

    /// The dots for the last propagated moment, in the same order as the Tracked Objects.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn summary(&self, at: JulianDate) -> Summary {
        let count = |kind| {
            self.objects
                .iter()
                .filter(|object| object.kind == kind)
                .count()
        };
        Summary {
            satellites: count(ObjectKind::Satellite),
            rocket_bodies: count(ObjectKind::RocketBody),
            debris: count(ObjectKind::Debris),
            // Dots hidden by the filter are not missing a Position.
            without_position: self
                .instances
                .iter()
                .zip(&self.visible)
                .filter(|(instance, visible)| **visible && instance.is_hidden())
                .count(),
            shown: self.matches.len(),
            median_element_age_days: self.median_epoch.map(|epoch| at.0 - epoch.0),
            propagation_ms: self.last_propagation.as_secs_f32() * 1000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use orion_core::filter::Group;

    use crate::feed::{load_snapshot, load_snapshot_catalogue, load_snapshot_groups};

    fn snapshot() -> Tracked {
        Tracked::new(
            load_snapshot().expect("a readable Snapshot"),
            load_snapshot_catalogue().expect("readable Catalogue Records"),
            load_snapshot_groups().expect("readable Group lists"),
        )
    }

    /// A moment inside the period the Snapshot was captured in.
    fn snapshot_moment() -> JulianDate {
        JulianDate::from_utc(2026, 11, 24, 21, 0, 0.0)
    }

    fn radius(instance: &Instance) -> f32 {
        instance.position().length()
    }

    fn station(tracked: &Tracked) -> usize {
        tracked
            .index_of(25_544)
            .expect("the International Space Station is in the Snapshot")
    }

    #[test]
    fn snapshot_loads_every_kind_without_duplicates() {
        let tracked = snapshot();
        let summary = tracked.summary(snapshot_moment());
        assert!(summary.satellites > 15_000, "{summary:?}");
        assert!(summary.rocket_bodies > 2_000, "{summary:?}");
        assert!(summary.debris > 10_000, "{summary:?}");

        let numbers: HashSet<u32> = tracked
            .objects
            .iter()
            .map(|object| object.catalogue_number)
            .collect();
        assert_eq!(numbers.len(), tracked.objects.len());

        // Most Element Sets were issued within the day before the Snapshot was taken.
        let median_age = summary.median_element_age_days.expect("a median Epoch");
        assert!(median_age.abs() < 1.0, "{summary:?}");
    }

    #[test]
    fn nearly_every_object_has_a_position_above_the_surface() {
        let mut tracked = snapshot();
        tracked.propagate(snapshot_moment(), Highlight::default());

        let instances = tracked.instances();
        assert_eq!(instances.len(), tracked.objects.len());

        let above_surface = instances
            .iter()
            .filter(|instance| !instance.is_hidden() && radius(instance) > 1.0)
            .count();
        assert!(
            above_surface as f64 > 0.999 * instances.len() as f64,
            "only {above_surface} of {} are above the surface",
            instances.len()
        );
    }

    #[test]
    fn the_space_station_is_where_a_space_station_should_be() {
        let mut tracked = snapshot();
        tracked.propagate(snapshot_moment(), Highlight::default());

        let index = tracked
            .objects
            .iter()
            .position(|object| object.catalogue_number == 25_544)
            .expect("the International Space Station is in the Snapshot");
        let station = &tracked.instances()[index];

        // It flies between roughly 370 and 460 km up, and its orbit is tilted 51.6 degrees,
        // so it never passes over a latitude higher than that.
        let altitude_km = (radius(station) as f64 - 1.0) * EARTH_EQUATORIAL_RADIUS_KM;
        assert!((370.0..460.0).contains(&altitude_km), "{altitude_km} km");

        let latitude_deg = (station.position().z / radius(station)).asin().to_degrees();
        assert!(latitude_deg.abs() < 51.8, "{latitude_deg} degrees");
    }

    #[test]
    fn highlighted_objects_are_marked() {
        let mut tracked = snapshot();
        let index = station(&tracked);

        tracked.propagate(snapshot_moment(), Highlight::default());
        let plain = tracked.instances()[index];
        let neighbour = tracked.instances()[index + 1];

        let highlight = Highlight {
            selected: Some(index),
            hovered: None,
        };
        tracked.propagate(snapshot_moment(), highlight);
        let selected = tracked.instances()[index];

        assert_eq!(selected.position(), plain.position());
        assert_ne!(selected, plain);
        // No other dot changes.
        assert_eq!(tracked.instances()[index + 1], neighbour);
    }

    #[test]
    fn details_describe_the_space_station() {
        let tracked = snapshot();
        let details = tracked
            .details(station(&tracked), snapshot_moment())
            .expect("details");

        assert!(details.name.contains("ISS"), "{}", details.name);
        assert_eq!(details.catalogue_number, 25_544);
        assert_eq!(details.designator, "1998-067A");
        assert_eq!(details.kind, ObjectKind::Satellite);
        assert!(!details.stale);
        assert!(
            (92.0..94.0).contains(&details.period_minutes),
            "{details:?}"
        );
        assert!((details.inclination_deg - 51.6).abs() < 0.1, "{details:?}");
        assert!(details.perigee_altitude_km <= details.apogee_altitude_km);

        let record = details.record.expect("a Catalogue Record");
        assert_eq!(record.owner, "International Space Station (ISS)");
        assert_eq!(record.launch_date, "1998-11-20");
        assert!(record.launch_site.ends_with("(TYMSC)"), "{record:?}");
        assert!(record.launch_site.contains("Kazakhstan"), "{record:?}");

        let now = details.now.expect("a Position");
        assert!((370.0..460.0).contains(&now.over.altitude_km), "{now:?}");
        // A low circular orbit is travelled at about 7.66 km/s.
        assert!((now.speed_km_s - 7.66).abs() < 0.05, "{now:?}");
    }

    #[test]
    fn element_sets_go_stale_after_thirty_days() {
        let mut tracked = snapshot();
        let index = station(&tracked);
        let later = JulianDate(snapshot_moment().0 + 40.0);

        assert!(tracked.details(index, later).expect("details").stale);

        // A stale dot differs from a fresh one only in how it is marked.
        tracked.propagate(later, Highlight::default());
        let stale = tracked.instances()[index];
        assert!(!stale.is_hidden());
        let fresh = Instance::new(
            stale.position(),
            kind_color(ObjectKind::Satellite),
            Marks::default(),
        );
        assert_ne!(stale, fresh);
    }

    #[test]
    fn paths_follow_the_orbit_and_the_ground() {
        let tracked = snapshot();
        let index = station(&tracked);

        let orbit_path = tracked.orbit_path(index, snapshot_moment());
        assert_eq!(orbit_path.len(), PATH_SAMPLES);
        for point in &orbit_path {
            let altitude_km = (point.length() as f64 - 1.0) * EARTH_EQUATORIAL_RADIUS_KM;
            assert!((370.0..460.0).contains(&altitude_km), "{altitude_km} km");
        }

        let ground_track = tracked.ground_track(index, snapshot_moment());
        assert_eq!(ground_track.len(), PATH_SAMPLES);
        for point in &ground_track {
            assert!((point.length() - 1.0).abs() < 1e-5);
        }
        // Both start beneath the same spot.
        assert!(
            orbit_path[0]
                .normalize()
                .distance(ground_track[0].normalize())
                < 1e-5
        );
    }

    #[test]
    fn a_refresh_without_catalogue_records_keeps_the_ones_on_hand() {
        let mut tracked = snapshot();
        let index = station(&tracked);
        let before = tracked.details(index, snapshot_moment()).unwrap().record;
        assert!(before.is_some());

        tracked.replace(load_snapshot().unwrap(), None, None);
        let index = station(&tracked);
        assert_eq!(
            tracked.details(index, snapshot_moment()).unwrap().record,
            before
        );

        // New records replace the old; an object missing from them has none.
        tracked.replace(
            load_snapshot().unwrap(),
            Some(CatalogueRecords::new()),
            None,
        );
        let index = station(&tracked);
        assert_eq!(
            tracked.details(index, snapshot_moment()).unwrap().record,
            None
        );
    }

    #[test]
    fn codes_without_a_known_name_are_shown_as_they_are() {
        assert_eq!(spell_out("US", Some("United States")), "United States (US)");
        assert_eq!(spell_out("XYZ", None), "XYZ");
    }

    #[test]
    fn everything_is_shown_until_a_filter_is_set() {
        let tracked = snapshot();
        assert_eq!(tracked.matches().len(), tracked.len());
        assert_eq!(tracked.summary(snapshot_moment()).shown, tracked.len());

        // Matches are in name order.
        let names: Vec<&str> = tracked
            .matches()
            .iter()
            .map(|index| tracked.object(*index).unwrap().name.as_str())
            .collect();
        assert!(names.is_sorted());
    }

    #[test]
    fn a_group_filter_shows_only_its_members() {
        let mut tracked = snapshot();
        tracked.set_filter(Filter {
            group: Some(Group::SpaceStations),
            ..Filter::default()
        });

        // CelesTrak lists 17 space station objects; all are in the Snapshot.
        assert_eq!(tracked.matches().len(), 17);
        assert!(tracked.matches().contains(&station(&tracked)));

        tracked.propagate(snapshot_moment(), Highlight::default());
        let drawn = tracked
            .instances()
            .iter()
            .filter(|instance| !instance.is_hidden())
            .count();
        assert_eq!(drawn, 17);
        assert_eq!(tracked.summary(snapshot_moment()).without_position, 0);
    }

    #[test]
    fn searching_finds_objects_by_name() {
        let mut tracked = snapshot();
        tracked.set_filter(Filter {
            text: "noaa 2".to_owned(),
            ..Filter::default()
        });
        let names: Vec<&str> = tracked
            .matches()
            .iter()
            .map(|index| tracked.object(*index).unwrap().name.as_str())
            .collect();
        assert!(names.contains(&"NOAA 20 (JPSS-1)"), "{names:?}");
        assert!(
            names.iter().all(|name| name.contains("NOAA 2")),
            "{names:?}"
        );
    }

    #[test]
    fn the_selected_object_stays_on_show_when_filtered_out() {
        let mut tracked = snapshot();
        let index = station(&tracked);
        tracked.set_filter(Filter {
            satellites: false,
            ..Filter::default()
        });
        assert!(!tracked.matches().contains(&index));

        tracked.propagate(snapshot_moment(), Highlight::default());
        assert!(tracked.instances()[index].is_hidden());

        let highlight = Highlight {
            selected: Some(index),
            hovered: None,
        };
        tracked.propagate(snapshot_moment(), highlight);
        assert!(!tracked.instances()[index].is_hidden());
    }

    #[test]
    fn a_refresh_keeps_the_filter() {
        let mut tracked = snapshot();
        tracked.set_filter(Filter {
            group: Some(Group::Starlink),
            ..Filter::default()
        });
        let before = tracked.matches().len();
        assert!(before > 10_000);

        tracked.replace(load_snapshot().unwrap(), None, None);
        assert_eq!(tracked.matches().len(), before);
    }
}
