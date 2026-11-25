//! The Layers under the Energy Theme: where energy is made, dug, stored and moved.
//!
//! Each Layer is one kind of thing, and its classes are what it runs on, digs or
//! carries, so the filter doubles as a filter by fuel. All come from the project's
//! Extracts of OpenStreetMap, so each is as complete as volunteers have made it.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::places;

use crate::feature_layer::{Class, Show, Spec};

/// The credit OpenStreetMap's licence asks to be shown wherever its data is.
const OPENSTREETMAP: &str = "© OpenStreetMap contributors";
const VOLUNTEER_MAPPED: &str =
    "As mapped by OpenStreetMap's volunteers. Places nobody has mapped are missing.";

/// Power plants, sized by capacity where that is recorded.
pub static POWER_PLANTS: Spec = Spec {
    name: "Power plants",
    theme: Theme::Energy,
    listing: places::POWER_PLANTS,
    classes: &[
        Class::new("Solar", [255, 213, 79]),
        Class::new("Wind", [128, 222, 234]),
        Class::new("Hydro", [79, 148, 247]),
        Class::new("Gas", [255, 138, 101]),
        Class::new("Coal", [161, 136, 127]),
        Class::new("Oil", [141, 110, 99]),
        Class::new("Nuclear", [206, 147, 216]),
        Class::new("Biomass or waste", [174, 213, 129]),
        Class::new("Geothermal", [240, 98, 146]),
        Class::new("Other or not recorded", [176, 190, 197]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    // A megawatt is the usual dot; a gigawatt plant is two and a half times it.
    dot_scale: |plant| 1.0 + plant.size.max(1.0).log10().min(4.0) * 0.5,
    note: "As mapped by OpenStreetMap's volunteers. Plants in use, without those \
           planned or closed; capacity is missing for many.",
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
};

/// Mines and quarries whose product is recorded. Stone, sand and gravel pits far
/// outnumber the rest, so they wait in the filter.
pub static MINES: Spec = Spec {
    name: "Mines",
    theme: Theme::Energy,
    listing: places::MINES,
    classes: &[
        Class::new("Coal", [161, 136, 127]),
        Class::new("Iron", [229, 115, 115]),
        Class::new("Copper", [255, 183, 77]),
        Class::new("Gold, silver and gems", [255, 241, 118]),
        Class::new("Lithium", [128, 222, 234]),
        Class::new("Other minerals", [186, 104, 200]),
        Class::hidden("Stone, sand and gravel", [189, 189, 189]),
        Class::hidden("Product not recorded", [140, 140, 140]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.3,
    note: "As mapped by OpenStreetMap's volunteers. What a mine digs is recorded for \
           only some, and lithium in particular is thinly mapped.",
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
};

pub static OIL_AND_GAS_SITES: Spec = Spec {
    name: "Refineries and oil sites",
    theme: Theme::Energy,
    listing: places::OIL_AND_GAS_SITES,
    classes: &[
        Class::new("Refineries", [255, 112, 67]),
        Class::new("Other oil and gas sites", [255, 171, 145]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |site| if site.class == 0 { 1.8 } else { 1.2 },
    note: VOLUNTEER_MAPPED,
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
};

/// Oil and gas platforms at sea.
pub static OFFSHORE_PLATFORMS: Spec = Spec {
    name: "Offshore platforms",
    theme: Theme::Energy,
    listing: places::OFFSHORE_PLATFORMS,
    classes: &[Class::new("Offshore platforms", [215, 168, 110])],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.3,
    note: VOLUNTEER_MAPPED,
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
};

/// Oil, gas and fuel pipelines, drawn as their routes.
pub static PIPELINES: Spec = Spec {
    name: "Pipelines",
    theme: Theme::Energy,
    listing: places::PIPELINES,
    classes: &[
        Class::new("Gas", [255, 238, 88]),
        Class::new("Oil", [255, 112, 67]),
        Class::new("Fuel and other products", [192, 202, 51]),
    ],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.0,
    note: "As mapped by OpenStreetMap's volunteers, in the pieces they were mapped in. \
           Many pipelines are missing or broken off, buried ones most of all.",
    attribution: OPENSTREETMAP,
    cluster: false,
    routes: true,
};

pub static DAMS: Spec = Spec {
    name: "Dams",
    theme: Theme::Energy,
    listing: places::DAMS,
    classes: &[Class::new("Dams", [100, 181, 246])],
    coverage: TimeCoverage::AnyMoment,
    show: Show::Always,
    dot_scale: |_| 1.3,
    note: "As mapped by OpenStreetMap's volunteers. Named dams whose height is recorded \
           or that are otherwise of note, whether or not they make power.",
    attribution: OPENSTREETMAP,
    cluster: true,
    routes: false,
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 6] = [
    &POWER_PLANTS,
    &PIPELINES,
    &OIL_AND_GAS_SITES,
    &OFFSHORE_PLATFORMS,
    &MINES,
    &DAMS,
];
