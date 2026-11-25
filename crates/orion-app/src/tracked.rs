//! The Tracked Objects on show and their Positions for the current frame.

use std::time::{Duration, Instant};

use glam::DVec3;
use orion_core::EARTH_EQUATORIAL_RADIUS_KM;
use orion_core::catalogue::{ObjectKind, TrackedObject};
use orion_core::time::JulianDate;
use rayon::prelude::*;

use crate::objects::Instance;

pub struct Tracked {
    objects: Vec<TrackedObject>,
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
    /// The Element Age, in days, that half the Element Sets exceed.
    pub median_element_age_days: Option<f64>,
    pub propagation_ms: f32,
}

impl Tracked {
    pub fn new(objects: Vec<TrackedObject>) -> Self {
        // The median, not the newest: a few Element Sets are published with an Epoch
        // days in the future, which would make the whole set look fresher than it is.
        let mut epochs: Vec<f64> = objects
            .iter()
            .map(|object| object.element_set.epoch().0)
            .collect();
        epochs.sort_by(f64::total_cmp);
        let median_epoch = epochs.get(epochs.len() / 2).copied().map(JulianDate);

        Self {
            objects,
            instances: Vec::new(),
            median_epoch,
            last_propagation: Duration::ZERO,
        }
    }

    /// Swaps in a new set of Tracked Objects, for example after a refresh.
    pub fn replace(&mut self, objects: Vec<TrackedObject>) {
        *self = Self::new(objects);
    }

    /// Computes every Tracked Object's Position at the given moment.
    pub fn propagate(&mut self, at: JulianDate) {
        let started = Instant::now();
        self.objects
            .par_iter()
            .map(|object| match object.element_set.position(at) {
                Some(position_km) => Instance::new(
                    (DVec3::from_array(position_km) / EARTH_EQUATORIAL_RADIUS_KM).as_vec3(),
                    object.kind,
                ),
                None => Instance::HIDDEN,
            })
            .collect_into_vec(&mut self.instances);
        self.last_propagation = started.elapsed();
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
            without_position: self
                .instances
                .iter()
                .filter(|instance| instance.is_hidden())
                .count(),
            median_element_age_days: self.median_epoch.map(|epoch| at.0 - epoch.0),
            propagation_ms: self.last_propagation.as_secs_f32() * 1000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::feed::load_snapshot;

    fn snapshot() -> Tracked {
        Tracked::new(load_snapshot().expect("a readable Snapshot"))
    }

    /// A moment inside the period the Snapshot was captured in.
    fn snapshot_moment() -> JulianDate {
        JulianDate::from_utc(2026, 11, 24, 21, 0, 0.0)
    }

    fn radius(instance: &Instance) -> f32 {
        glam::Vec3::from_array(
            bytemuck::cast::<_, [f32; 4]>(*instance)[..3]
                .try_into()
                .unwrap(),
        )
        .length()
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
        tracked.propagate(snapshot_moment());

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
        tracked.propagate(snapshot_moment());

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

        let [_, _, z, _] = bytemuck::cast::<_, [f32; 4]>(*station);
        let latitude_deg = (z / radius(station)).asin().to_degrees();
        assert!(latitude_deg.abs() < 51.8, "{latitude_deg} degrees");
    }
}
