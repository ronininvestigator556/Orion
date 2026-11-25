//! The Layers under the Weather Theme that are lists of places: earthquakes, fire
//! detections, tropical storms and U.S. weather alerts.
//!
//! Each Layer keeps to its own family of colours, so its dots can be told from the
//! others' when several are on: earthquakes yellow to red, fire detections flame
//! orange, storms white and aqua, alerts pink and violet.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::weather;

use crate::feature_layer::{Class, Show, Spec};

/// Earthquakes of the past week. With the Simulation Clock moved back, the Layer shows
/// the week before that moment, as far as the listing reaches.
pub static EARTHQUAKES: Spec = Spec {
    name: "Earthquakes",
    theme: Theme::Weather,
    listing: weather::EARTHQUAKES,
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
};

/// Hot spots seen from orbit in the past day.
pub static FIRE_DETECTIONS: Spec = Spec {
    name: "Fire detections",
    theme: Theme::Weather,
    listing: weather::FIRE_DETECTIONS,
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
};

/// Tropical storms being tracked now.
pub static STORMS: Spec = Spec {
    name: "Tropical storms",
    theme: Theme::Weather,
    listing: weather::STORMS,
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
};

/// Weather alerts in force in the United States.
pub static ALERTS: Spec = Spec {
    name: "U.S. weather alerts",
    theme: Theme::Weather,
    listing: weather::ALERTS,
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
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 4] = [&EARTHQUAKES, &FIRE_DETECTIONS, &STORMS, &ALERTS];
