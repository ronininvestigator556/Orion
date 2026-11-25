//! Themes, Time Coverage, and what a Feature shows about itself.
//!
//! A Layer is one kind of thing from one kind of Source. The types here are the parts
//! of that idea that hold for every Layer, whatever it shows and however it is drawn.

use crate::time::JulianDate;

/// A heading under which related Layers are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Theme {
    Orbit,
    Places,
    Energy,
    /// What is happening on the roads themselves: incidents and congestion.
    Roads,
    /// Vehicles shown moving, such as aircraft and ships.
    Traffic,
    /// Pictures of the sky and sea.
    Weather,
    /// Earthquakes, storms and official warnings.
    Hazards,
    /// Protests and strikes as reported.
    Unrest,
    Imported,
}

impl Theme {
    /// Every Theme, in the order they are listed.
    pub const ALL: [Self; 9] = [
        Self::Orbit,
        Self::Places,
        Self::Energy,
        Self::Roads,
        Self::Traffic,
        Self::Weather,
        Self::Hazards,
        Self::Unrest,
        Self::Imported,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Orbit => "Orbit",
            Self::Places => "Places",
            Self::Energy => "Energy",
            Self::Roads => "Roads",
            Self::Traffic => "Traffic",
            Self::Weather => "Weather",
            Self::Hazards => "Hazards",
            Self::Unrest => "Unrest",
            Self::Imported => "Imported",
        }
    }
}

/// How far the Simulation Clock may be from real time and still count as the present:
/// five minutes, in days. Live data is never fresher than that anyway.
pub const PRESENT_TOLERANCE_DAYS: f64 = 5.0 / 1_440.0;

/// The moments a Layer is able to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeCoverage {
    /// Any moment the Simulation Clock can reach, as for things whose position is
    /// computed or which do not move.
    AnyMoment,
    /// Only the present, as for live positions with no history kept.
    PresentOnly,
    /// The present and this many days before it, as for imagery with an archive.
    PresentAndPast { days: f64 },
}

impl TimeCoverage {
    /// True if a Layer with this coverage can show `moment`, given the real time now.
    pub fn covers(self, moment: JulianDate, real_now: JulianDate) -> bool {
        // Positive when the moment is in the past.
        let days_ago = real_now.0 - moment.0;
        match self {
            Self::AnyMoment => true,
            Self::PresentOnly => days_ago.abs() <= PRESENT_TOLERANCE_DAYS,
            Self::PresentAndPast { days } => (-PRESENT_TOLERANCE_DAYS..=days).contains(&days_ago),
        }
    }
}

/// One line of a Feature card: what is being stated, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardRow {
    pub label: String,
    pub value: String,
}

impl CardRow {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// What a selected Feature shows about itself, in a form any Layer can fill in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FeatureCard {
    pub title: String,
    /// Groups of related rows, shown with a divider between them.
    pub sections: Vec<Vec<CardRow>>,
    /// Longer text about the Feature, a paragraph at a time.
    pub details: Vec<String>,
    /// Things the reader should be warned about, such as stale data.
    pub notices: Vec<String>,
    /// Who published the information.
    pub source: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_now() -> JulianDate {
        JulianDate::from_utc(2026, 11, 25, 12, 0, 0.0)
    }

    fn days_from_now(days: f64) -> JulianDate {
        JulianDate(real_now().0 + days)
    }

    #[test]
    fn themes_are_listed_in_their_order_under_their_names() {
        let names: Vec<&str> = Theme::ALL.iter().map(|theme| theme.name()).collect();
        assert_eq!(
            names,
            [
                "Orbit", "Places", "Energy", "Roads", "Traffic", "Weather", "Hazards", "Unrest",
                "Imported"
            ]
        );
    }

    #[test]
    fn anything_computed_or_fixed_covers_every_moment() {
        for days in [-14.0, -1.0, 0.0, 14.0] {
            assert!(TimeCoverage::AnyMoment.covers(days_from_now(days), real_now()));
        }
    }

    #[test]
    fn live_data_covers_only_the_present() {
        let live = TimeCoverage::PresentOnly;
        assert!(live.covers(real_now(), real_now()));
        // A clock a couple of minutes off is still showing the present.
        assert!(live.covers(days_from_now(2.0 / 1_440.0), real_now()));
        assert!(live.covers(days_from_now(-2.0 / 1_440.0), real_now()));
        assert!(!live.covers(days_from_now(-1.0 / 24.0), real_now()));
        assert!(!live.covers(days_from_now(1.0 / 24.0), real_now()));
    }

    #[test]
    fn an_archive_covers_the_present_and_its_past_but_not_the_future() {
        let archive = TimeCoverage::PresentAndPast { days: 3.0 };
        assert!(archive.covers(real_now(), real_now()));
        assert!(archive.covers(days_from_now(-2.9), real_now()));
        assert!(!archive.covers(days_from_now(-3.1), real_now()));
        assert!(!archive.covers(days_from_now(0.5), real_now()));
    }
}
