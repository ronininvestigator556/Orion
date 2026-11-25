//! The Layers under the Roads Theme: Road Incidents, a country at a time.
//!
//! Every country publishes its roads' troubles in its own way and covers its own
//! part of them, so each is a Layer of its own that says what it leaves out, and
//! nothing is fetched for a country nobody has switched on.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::roads;

use crate::feature_layer::{Class, Show, Spec};

/// The kinds of Road Incident, the same colours in every country.
const KINDS: [Class; 4] = [
    Class::new("Accident", [244, 67, 54]),
    Class::new("Closure or restriction", [255, 112, 67]),
    Class::new("Roadworks", [255, 202, 40]),
    Class::new("Other", [144, 164, 174]),
];

pub static FRANCE: Spec = Spec {
    name: "Road incidents: France",
    theme: Theme::Roads,
    listing: roads::FRANCE,
    classes: &KINDS,
    coverage: TimeCoverage::PresentOnly,
    show: Show::InForce,
    dot_scale: |_| 1.8,
    note: "The roads the French state runs itself. Most toll motorways are run by \
           companies that publish nothing here, so they are missing, and few accidents \
           are listed: most of this is roadworks and lane restrictions. Details are in \
           French, as published.",
    attribution: "Road incidents in France: Bison Futé, Licence Ouverte 2.0",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

pub static UNITED_STATES: Spec = Spec {
    name: "Road works: United States",
    theme: Theme::Roads,
    listing: roads::UNITED_STATES,
    // Work zones are never accidents or anything else, so they have two kinds of
    // their own.
    classes: &[
        Class::new("Road closed", [255, 112, 67]),
        Class::new("Roadworks", [255, 202, 40]),
    ],
    coverage: TimeCoverage::PresentOnly,
    show: Show::Always,
    dot_scale: |_| 1.6,
    note: "Work zones under way, from the states and cities that publish theirs without \
           a key: about twenty, so most of the country is blank. No state publishes its \
           accidents this way. Gathered once an hour.",
    attribution: "",
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

pub static FINLAND: Spec = Spec {
    name: "Road incidents: Finland",
    theme: Theme::Roads,
    listing: roads::FINLAND,
    classes: &KINDS,
    coverage: TimeCoverage::PresentOnly,
    show: Show::InForce,
    dot_scale: |_| 1.8,
    note: "Traffic announcements and roadworks in force now. Details are in Finnish, as \
           published.",
    attribution: "Road incidents in Finland: Fintraffic / digitraffic.fi, CC BY 4.0",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 3] = [&FRANCE, &UNITED_STATES, &FINLAND];

#[cfg(test)]
mod tests {
    use orion_data::roads::{incident_class, work_zone_class};

    use super::*;

    #[test]
    fn each_country_is_its_own_layer_under_roads_and_says_what_it_lacks() {
        let names: Vec<&str> = ALL.iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            [
                "Road incidents: France",
                "Road works: United States",
                "Road incidents: Finland"
            ]
        );
        for spec in ALL {
            assert_eq!(spec.theme, Theme::Roads, "{}", spec.name);
            assert_eq!(spec.coverage, TimeCoverage::PresentOnly, "{}", spec.name);
        }
        assert!(FRANCE.note.contains("toll motorways"));
        assert!(
            UNITED_STATES
                .note
                .contains("No state publishes its accidents")
        );
        // The two that ask for credit are given it.
        assert!(FRANCE.attribution.contains("Licence Ouverte"));
        assert!(FINLAND.attribution.contains("CC BY 4.0"));
    }

    #[test]
    fn the_kinds_are_in_the_order_the_readers_number_them() {
        let label = |spec: &Spec, class: u8| spec.classes[usize::from(class)].label;
        assert_eq!(label(&FRANCE, incident_class::ACCIDENT), "Accident");
        assert_eq!(
            label(&FRANCE, incident_class::CLOSURE),
            "Closure or restriction"
        );
        assert_eq!(label(&FRANCE, incident_class::ROADWORKS), "Roadworks");
        assert_eq!(label(&FINLAND, incident_class::OTHER), "Other");
        assert_eq!(
            label(&UNITED_STATES, work_zone_class::ROAD_CLOSED),
            "Road closed"
        );
        assert_eq!(
            label(&UNITED_STATES, work_zone_class::ROADWORKS),
            "Roadworks"
        );
    }
}
