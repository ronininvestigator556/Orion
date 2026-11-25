//! Layers of places that stay where they are: airports, and what the project's
//! Extracts carry from OpenStreetMap.
//!
//! As with the Hazards Layers, each keeps to its own family of colours: airports
//! white and grey, cameras blue, data centres teal, military sites olive.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::places::{self, airport_class};

use crate::feature_layer::{Class, Show, Spec};

/// The credit OpenStreetMap's licence asks to be shown wherever its data is.
const OPENSTREETMAP: &str = "© OpenStreetMap contributors";
const VOLUNTEER_MAPPED: &str =
    "As mapped by OpenStreetMap's volunteers. Places nobody has mapped are missing.";

/// Airports worldwide. Only the large and medium ones show until the filter is
/// changed, because the tens of thousands of airstrips and heliports bury them.
pub static AIRPORTS: Spec = Spec {
    name: "Airports",
    theme: Theme::Places,
    listing: places::AIRPORTS,
    classes: &[
        Class::new("Large airports", [255, 255, 255]),
        Class::new("Medium airports", [200, 210, 216]),
        Class::hidden("Small airports", [144, 164, 174]),
        Class::hidden("Heliports", [128, 150, 160]),
        Class::hidden("Seaplane bases", [128, 170, 180]),
        Class::hidden("Other landing places", [120, 130, 140]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |airport| match airport.class {
        airport_class::LARGE => 2.2,
        airport_class::MEDIUM => 1.5,
        _ => 1.0,
    },
    note: "Small airports, heliports and seaplane bases are in the filter.",
    attribution: "",
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

/// Cameras that read licence plates. A fixed piece of equipment at a mapped place;
/// nothing about what any camera has seen is shown, here or anywhere in Orion.
pub static CAMERAS: Spec = Spec {
    name: "Licence plate cameras",
    theme: Theme::Places,
    listing: places::CAMERAS,
    classes: &[
        Class::new("Made by Flock Safety", [68, 138, 255]),
        Class::new("Another named maker", [130, 177, 255]),
        Class::new("Maker not recorded", [92, 107, 192]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.0,
    note: "As mapped by OpenStreetMap's volunteers. Many cameras are not mapped, and \
           some that are mapped have since been removed.",
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

pub static DATA_CENTRES: Spec = Spec {
    name: "Data centres",
    theme: Theme::Places,
    listing: places::DATA_CENTRES,
    classes: &[Class::new("Data centres", [29, 233, 182])],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.6,
    note: VOLUNTEER_MAPPED,
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

pub static MILITARY_SITES: Spec = Spec {
    name: "Military sites",
    theme: Theme::Places,
    listing: places::MILITARY_SITES,
    classes: &[
        Class::new("Bases", [192, 202, 51]),
        Class::new("Airfields", [220, 231, 117]),
        Class::new("Naval bases", [139, 195, 74]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.5,
    note: VOLUNTEER_MAPPED,
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 4] = [&AIRPORTS, &CAMERAS, &DATA_CENTRES, &MILITARY_SITES];
