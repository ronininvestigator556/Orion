//! The Layers under the Conflict Theme: Armed Conflicts, for a view of where the
//! world's fighting is, the Conflict Events beneath them, and the Airspace Warnings
//! that an aviation authority publishes about conflict zones.
//!
//! Both come from the Uppsala Conflict Data Program, which is careful and so is
//! weeks behind: this is where there has been fighting lately, never where there is
//! fighting today.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::{conflict, warnings};

use crate::feature_layer::{Class, Show, Spec};

/// The three kinds of organised violence, the same colours in both Layers.
const KINDS: [Class; 3] = [
    Class::new("Fighting involving a state", [229, 57, 53]),
    Class::new("Fighting between armed groups", [255, 138, 101]),
    Class::new("Attacks on civilians", [186, 104, 200]),
];
const CREDIT: &str = "Conflict data: Uppsala Conflict Data Program, CC BY 4.0";

/// How much larger a mark is for the deaths it stands for: each tenfold step in
/// deaths adds the same amount, so one battle does not hide a hundred skirmishes.
fn scale_by_deaths(deaths: f32, smallest: f32, step: f32) -> f32 {
    smallest + step * (1.0 + deaths.max(0.0)).log10()
}

pub static ARMED_CONFLICTS: Spec = Spec {
    name: "Armed conflicts",
    theme: Theme::Conflict,
    listing: conflict::ARMED_CONFLICTS,
    classes: &KINDS,
    coverage: TimeCoverage::PresentOnly,
    show: Show::Always,
    dot_scale: |conflict| scale_by_deaths(conflict.size, 1.6, 0.9),
    note: "One mark for each conflict in each country it is fought in, placed in the \
           middle of its recorded incidents and larger the more people died. It covers \
           the last three months on record, which end three to eight weeks ago, and \
           counts only incidents in which someone was killed.",
    attribution: CREDIT,
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

pub static CONFLICT_EVENTS: Spec = Spec {
    name: "Conflict events",
    theme: Theme::Conflict,
    listing: conflict::CONFLICT_EVENTS,
    classes: &KINDS,
    coverage: TimeCoverage::PresentOnly,
    show: Show::Always,
    dot_scale: |event| scale_by_deaths(event.size, 1.0, 0.7),
    note: "Each recorded incident in which at least one person died, over the last three \
           months on record, which end three to eight weeks ago. Some are placed only \
           to a district or province, and the newest are still being checked: each \
           says so. Attacks on civilians include what is often called terrorism.",
    attribution: CREDIT,
    cluster: true,
    routes: false,
    ring_km: 0.0,
};

pub static AIRSPACE_WARNINGS: Spec = Spec {
    name: "Airspace warnings",
    theme: Theme::Conflict,
    listing: warnings::AIRSPACE_WARNINGS,
    classes: &[Class::new("In force", [79, 195, 247])],
    coverage: TimeCoverage::PresentOnly,
    show: Show::InForce,
    dot_scale: |_| 2.4,
    note: "The countries whose airspace Europe's aviation safety agency warns airlines \
           about because of a conflict. A warning speaks for the airspace, not for the \
           point it is marked at, and other authorities draw the lines differently.",
    attribution: "Airspace warnings: © European Union Aviation Safety Agency",
    cluster: false,
    routes: false,
    ring_km: 0.0,
};

/// Every Layer in this module, in the order they are listed.
pub static ALL: [&Spec; 3] = [&ARMED_CONFLICTS, &CONFLICT_EVENTS, &AIRSPACE_WARNINGS];

#[cfg(test)]
mod tests {
    use orion_data::conflict::conflict_class;

    use super::*;

    #[test]
    fn conflict_is_its_own_theme_with_the_high_level_view_first() {
        let names: Vec<&str> = ALL.iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            ["Armed conflicts", "Conflict events", "Airspace warnings"]
        );
        assert_eq!(AIRSPACE_WARNINGS.theme, Theme::Conflict);
        assert!(
            AIRSPACE_WARNINGS
                .attribution
                .contains("Aviation Safety Agency")
        );
        for spec in [&ARMED_CONFLICTS, &CONFLICT_EVENTS] {
            assert_eq!(spec.theme, Theme::Conflict, "{}", spec.name);
            // The Source asks to be credited, and both say how old the picture is.
            assert!(spec.attribution.contains("Uppsala"), "{}", spec.name);
            assert!(
                spec.note.contains("three to eight weeks ago"),
                "{}",
                spec.name
            );
        }
        // The kinds are in the order the reader numbers them.
        let label = |class: u8| KINDS[usize::from(class)].label;
        assert_eq!(
            label(conflict_class::INVOLVING_A_STATE),
            "Fighting involving a state"
        );
        assert_eq!(
            label(conflict_class::BETWEEN_ARMED_GROUPS),
            "Fighting between armed groups"
        );
        assert_eq!(
            label(conflict_class::AGAINST_CIVILIANS),
            "Attacks on civilians"
        );
    }

    #[test]
    fn a_mark_grows_with_each_tenfold_step_in_deaths() {
        let size = |deaths: f32| scale_by_deaths(deaths, 1.6, 0.9);
        assert_eq!(size(0.0), 1.6);
        assert!(size(9.0) > size(0.0));
        let step = size(999.0) - size(99.0);
        assert!((step - (size(99.0) - size(9.0))).abs() < 1e-4);
        // A war of eighteen thousand deaths is large, not enormous.
        assert!(
            size(18_045.0) > 5.0 && size(18_045.0) < 6.0,
            "{}",
            size(18_045.0)
        );
    }
}
