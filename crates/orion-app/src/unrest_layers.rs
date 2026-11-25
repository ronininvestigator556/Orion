//! The Layer under the Unrest Theme: protests, protests with violence and strikes as
//! the news reported them.
//!
//! It is a trial. The reports are sifted by machine from an index of the news that
//! is itself built by machine, and are often wrong, so the Layer is only listed when
//! Orion is started with `--experimental`.

use orion_core::layer::{Theme, TimeCoverage};
use orion_data::unrest;

use crate::feature_layer::{Class, Show, Spec};

/// How far from a report's mark its ring is drawn, in kilometres. A report is placed
/// by its town and no more exactly, so the ring says "somewhere about here".
const TOWN_KM: f64 = 8.0;

/// Unrest Reports of the past three days. What only one news outlet carried is
/// hidden until asked for.
pub static UNREST_REPORTS: Spec = Spec {
    name: "Protests and unrest",
    theme: Theme::Unrest,
    listing: unrest::UNREST_REPORTS,
    classes: &[
        Class::new("Protest", [174, 213, 129]),
        Class::new("Protest with violence", [255, 82, 82]),
        Class::new("Strike", [255, 241, 118]),
        Class::hidden("Carried by one outlet only", [158, 158, 158]),
    ],
    coverage: TimeCoverage::PresentAndPast { days: 7.0 },
    show: Show::Recent { days: 3.0 },
    // The more outlets carried it, the larger, up to a point.
    dot_scale: |report| 2.0 + 0.3 * report.size.min(6.0),
    note: "Experimental. Reports from the past three days, placed by town and drawn with a \
           ring because they are no more exact than that. They are put together by machine \
           from news articles and are often wrong; they never show where a crowd is now.",
    attribution: "Unrest reports: the GDELT Project, gdeltproject.org",
    cluster: false,
    routes: false,
    ring_km: TOWN_KM,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrest_is_its_own_theme_and_single_reports_are_hidden_until_asked_for() {
        assert_eq!(UNREST_REPORTS.theme, Theme::Unrest);
        let shown: Vec<(&str, bool)> = UNREST_REPORTS
            .classes
            .iter()
            .map(|class| (class.label, class.on))
            .collect();
        assert_eq!(
            shown,
            [
                ("Protest", true),
                ("Protest with violence", true),
                ("Strike", true),
                ("Carried by one outlet only", false),
            ]
        );
        // The classes are in the order the file's reader numbers them.
        assert_eq!(unrest::unrest_class::ONE_OUTLET, 3);
        assert!(UNREST_REPORTS.ring_km > 0.0);
        assert!(UNREST_REPORTS.note.starts_with("Experimental."));
        assert!(UNREST_REPORTS.attribution.contains("GDELT"));
    }
}
