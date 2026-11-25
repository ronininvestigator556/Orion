//! The Layers under the Hazards Theme: earthquakes, fire detections, tropical storms,
//! U.S. weather alerts and Travel Advisories.
//!
//! Each Layer keeps to its own family of colours, so its dots can be told from the
//! others' when several are on: earthquakes yellow to red, fire detections flame
//! orange, storms white and aqua, alerts pink and violet.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::{advisories, hazards};

use crate::feature_layer::{Class, Show, Spec};

/// Earthquakes of the past week. With the Simulation Clock moved back, the Layer shows
/// the week before that moment, as far as the listing reaches.
pub static EARTHQUAKES: Spec = Spec {
    name: "Earthquakes",
    theme: Theme::Hazards,
    listing: hazards::EARTHQUAKES,
    classes: &[
        Class::new("Magnitude under 4.5", [242, 201, 76]),
        Class::new("Magnitude 4.5 to 6", [242, 140, 40]),
        Class::new("Magnitude 6 and over", [235, 60, 60]),
    ],
    coverage: TimeCoverage::PresentAndPast { days: 7.0 },
    show: Show::Recent { days: 7.0 },
    // Magnitude 2.5 is the usual dot; each whole magnitude above adds most of another.
    dot_scale: |quake| 1.0 + (quake.size - 2.5).max(0.0) * 0.9,
    note: "Magnitude 2.5 and over, from the past seven days.",
    attribution: "",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Hot spots seen from orbit in the past day.
pub static FIRE_DETECTIONS: Spec = Spec {
    name: "Fire detections",
    theme: Theme::Hazards,
    listing: hazards::FIRE_DETECTIONS,
    classes: &[
        Class::new("High confidence", [255, 70, 20]),
        Class::new("Nominal confidence", [255, 138, 61]),
        Class::new("Low confidence", [170, 110, 80]),
    ],
    coverage: TimeCoverage::PresentAndPast { days: 1.0 },
    show: Show::Recent { days: 1.0 },
    dot_scale: |detection| if detection.size >= 100.0 { 1.6 } else { 1.0 },
    note: "Each is a hot spot about a kilometre across seen by a satellite in the past day. \
           It may be a wildfire, a planned burn, a gas flare or a volcano.",
    attribution: "",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Tropical storms being tracked now.
pub static STORMS: Spec = Spec {
    name: "Tropical storms",
    theme: Theme::Hazards,
    listing: hazards::STORMS,
    classes: &[
        Class::new("Hurricanes", [255, 255, 255]),
        Class::new("Tropical storms", [128, 255, 234]),
        Class::new("Depressions and others", [176, 190, 197]),
    ],
    coverage: TimeCoverage::PresentOnly,
    show: Show::InForce,
    dot_scale: |storm| 3.0 + (storm.size / 40.0).min(3.0),
    note: "Atlantic and eastern and central Pacific only. Outside the hurricane season \
           there is often nothing to show.",
    attribution: "",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Weather alerts in force in the United States.
pub static ALERTS: Spec = Spec {
    name: "U.S. weather alerts",
    theme: Theme::Hazards,
    listing: hazards::ALERTS,
    classes: &[
        Class::new("Extreme", [255, 20, 147]),
        Class::new("Severe", [224, 64, 251]),
        Class::new("Moderate", [179, 136, 255]),
        Class::new("Minor", [209, 196, 233]),
        Class::new("Severity not given", [150, 150, 160]),
    ],
    coverage: TimeCoverage::PresentOnly,
    show: Show::InForce,
    dot_scale: |alert| 1.4 + alert.size * 0.3,
    note: "United States only. An alert is drawn in the middle of the area it covers.",
    attribution: "",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// What the governments of the United States, the United Kingdom and Canada say
/// about travelling to each country: one mark a country, coloured by the United
/// States' level.
pub static TRAVEL_ADVISORIES: Spec = Spec {
    name: "Travel advisories",
    theme: Theme::Hazards,
    listing: advisories::TRAVEL_ADVISORIES,
    classes: &[
        Class::new("Level 1: usual precautions", [129, 199, 132]),
        Class::new("Level 2: increased caution", [255, 238, 88]),
        Class::new("Level 3: reconsider travel", [255, 152, 0]),
        Class::new("Level 4: do not travel", [229, 57, 53]),
        Class::new("No U.S. level", [158, 158, 158]),
    ],
    coverage: TimeCoverage::PresentOnly,
    show: Show::Always,
    // The graver the advice, the larger the mark.
    dot_scale: |advisory| 2.2 + 0.5 * advisory.size,
    note: "One mark for each country, coloured by the level the United States sets, with \
           the United Kingdom's and Canada's advice beside it. An advisory speaks for a \
           whole country, and names the parts it means in its own words: read the page.",
    attribution: "Travel advisories: U.S. Department of State; UK FCDO, Open Government \
                  Licence v3.0; Global Affairs Canada, Open Government Licence - Canada",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 5] = [
    &EARTHQUAKES,
    &FIRE_DETECTIONS,
    &STORMS,
    &ALERTS,
    &TRAVEL_ADVISORIES,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_earthquake_a_storm_or_a_warning_is_a_hazard_not_weather() {
        let names: Vec<&str> = ALL.iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            [
                "Earthquakes",
                "Fire detections",
                "Tropical storms",
                "U.S. weather alerts",
                "Travel advisories"
            ]
        );
        for spec in ALL {
            assert_eq!(spec.theme, Theme::Hazards, "{}", spec.name);
        }
    }

    #[test]
    fn a_travel_advisory_is_coloured_by_the_united_states_level() {
        let labels: Vec<&str> = TRAVEL_ADVISORIES
            .classes
            .iter()
            .map(|class| class.label)
            .collect();
        assert_eq!(
            labels,
            [
                "Level 1: usual precautions",
                "Level 2: increased caution",
                "Level 3: reconsider travel",
                "Level 4: do not travel",
                "No U.S. level"
            ]
        );
        // The classes are in the order the file's reader numbers them.
        assert_eq!(advisories::advisory_class::LEVEL_4, 3);
        assert_eq!(advisories::advisory_class::NO_US_LEVEL, 4);
        // Each government asks to be credited, two of them by licence.
        for credit in [
            "Department of State",
            "Open Government Licence v3.0",
            "Canada",
        ] {
            assert!(TRAVEL_ADVISORIES.attribution.contains(credit), "{credit}");
        }
    }
}
