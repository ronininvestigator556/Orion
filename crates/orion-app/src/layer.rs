//! Layers: what the app shows on the globe, one kind of thing at a time.

use std::ops::Range;
use std::time::SystemTime;

use orion_core::layer::{FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;

use crate::feed::Refresh;
use crate::lines::Segment;
use crate::objects::Instance;
use crate::orbit_layer::OrbitLayer;
pub use crate::tracked::Highlight;

/// One Feature: which Layer it is on, and where it comes in that Layer's own order.
///
/// A Feature's index can change when its Layer's data is refreshed. To keep hold of one
/// across a refresh, remember its key from [`Layer::stable_key`] as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureRef {
    /// Position of the Layer in [`Layers`].
    pub layer: usize,
    pub index: usize,
}

/// A switch a Layer offers for the selected Feature, such as showing its Ground Track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggle {
    pub label: &'static str,
    pub on: bool,
}

/// One kind of thing from one kind of Source, shown or hidden as a whole.
/// Where a Layer's data came from and how fresh it is.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerStatus {
    /// A line of text for the sidebar, such as "From CelesTrak, fetched 3.2 hours ago".
    pub text: String,
    /// Whether fresh data can be fetched right now; `None` for a Layer with nothing to fetch.
    pub refresh: Option<Refresh>,
}

pub trait Layer {
    fn name(&self) -> &'static str;
    fn theme(&self) -> Theme;
    fn time_coverage(&self) -> TimeCoverage;
    fn is_on(&self) -> bool;
    fn set_on(&mut self, on: bool);

    /// The colour that stands for the Layer in the sidebar, as linear colour with
    /// alpha from 0 to 255.
    fn swatch(&self) -> [u8; 4];

    /// How many Features pass the Layer's filter and the search, and how many it has in all.
    fn counts(&self) -> (usize, usize);

    fn status(&self, now: SystemTime) -> LayerStatus;

    /// Starts fetching fresh data, if the Layer has a Source to ask and may ask it now.
    fn request_refresh(&mut self, _now: SystemTime) {}

    /// Narrows the Layer to Features matching what is typed in the search box. Empty
    /// text shows everything the Layer's own filter allows.
    fn set_search(&mut self, text: &str);

    /// The first `limit` Features on show, in name order, each with a line naming it.
    fn found(&self, limit: usize) -> Vec<(usize, String)>;

    /// Draws the Layer's own filter controls in the sidebar, if it has any.
    fn filter_ui(&mut self, _ui: &mut egui::Ui) {}

    /// Works out where every Feature is at `at` and how each should look. Called once
    /// per frame while the Layer is on and can show that moment.
    fn update(&mut self, at: JulianDate, highlight: Highlight);

    /// One dot per Feature, in Feature order, as of the last `update`. A Feature that
    /// is not to be drawn is [`Instance::HIDDEN`], so indexes still line up.
    fn dots(&self) -> &[Instance];

    /// Appends the lines this Layer draws for the frame, such as the selected
    /// Feature's Orbit Path.
    fn lines(&self, selected: Option<usize>, at: JulianDate, out: &mut Vec<Segment>);

    fn feature_name(&self, index: usize) -> Option<&str>;

    /// What a Feature shows about itself when selected.
    fn card(&self, index: usize, at: JulianDate) -> Option<FeatureCard>;

    /// A number that identifies a Feature for as long as it exists, across refreshes.
    fn stable_key(&self, index: usize) -> Option<u64>;

    /// The current index of the Feature with this key, if it is still on the Layer.
    fn index_of(&self, key: u64) -> Option<usize>;

    /// Switches offered for the selected Feature, with their current settings.
    fn toggles(&self) -> Vec<Toggle> {
        Vec::new()
    }

    /// Applies switches as changed by the user, in the order `toggles` gave them.
    fn set_toggles(&mut self, _toggles: &[Toggle]) {}

    /// True while the Layer is on and still waiting for its first data to arrive.
    fn is_loading(&self) -> bool {
        false
    }

    /// Credit that must stay on screen while the Layer is on, where its Source asks
    /// for that.
    fn attribution(&self) -> Option<&'static str> {
        None
    }
}

/// Every Layer the app has, and their dots gathered into one list for drawing and picking.
pub struct Layers {
    /// The first Layer: everything in orbit. Kept by name because its filter and
    /// refresh controls still have panels of their own.
    pub orbit: OrbitLayer,
    others: Vec<Box<dyn Layer>>,
    /// The dots of every Layer that is on, one Layer after another.
    dots: Vec<Instance>,
    /// Which stretch of `dots` belongs to each Layer; empty for a Layer not on show.
    ranges: Vec<Range<usize>>,
}

impl Layers {
    pub fn new(orbit: OrbitLayer) -> Self {
        Self {
            orbit,
            others: Vec::new(),
            dots: Vec::new(),
            ranges: Vec::new(),
        }
    }

    /// Adds a Layer after the ones already present.
    pub fn add(&mut self, layer: Box<dyn Layer>) {
        self.others.push(layer);
    }

    pub fn len(&self) -> usize {
        1 + self.others.len()
    }

    pub fn get(&self, layer: usize) -> Option<&dyn Layer> {
        match layer {
            0 => Some(&self.orbit),
            other => self.others.get(other - 1).map(|layer| &**layer),
        }
    }

    pub fn get_mut(&mut self, layer: usize) -> Option<&mut dyn Layer> {
        match layer {
            0 => Some(&mut self.orbit),
            other => self
                .others
                .get_mut(other - 1)
                .map(|layer| &mut **layer as &mut dyn Layer),
        }
    }

    /// Brings every Layer up to the moment `at` and gathers their dots.
    ///
    /// A Layer contributes nothing while it is off, or while the Simulation Clock is
    /// somewhere its Time Coverage does not reach.
    pub fn update(
        &mut self,
        at: JulianDate,
        real_now: JulianDate,
        selected: Option<FeatureRef>,
        hovered: Option<FeatureRef>,
    ) {
        self.dots.clear();
        self.ranges.clear();
        for index in 0..self.len() {
            let start = self.dots.len();
            let on_this_layer = |feature: Option<FeatureRef>| {
                feature
                    .filter(|feature| feature.layer == index)
                    .map(|feature| feature.index)
            };
            let highlight = Highlight {
                selected: on_this_layer(selected),
                hovered: on_this_layer(hovered),
            };
            // Split the borrow: the Layer is read while the shared list is written.
            let layer: &mut dyn Layer = match index {
                0 => &mut self.orbit,
                other => &mut *self.others[other - 1],
            };
            if layer.is_on() && layer.time_coverage().covers(at, real_now) {
                layer.update(at, highlight);
                self.dots.extend_from_slice(layer.dots());
            }
            self.ranges.push(start..self.dots.len());
        }
    }

    /// Every dot on show, as of the last `update`.
    pub fn dots(&self) -> &[Instance] {
        &self.dots
    }

    /// The Feature a dot in [`Self::dots`] stands for.
    pub fn feature_at(&self, dot: usize) -> Option<FeatureRef> {
        let layer = self.ranges.iter().position(|range| range.contains(&dot))?;
        Some(FeatureRef {
            layer,
            index: dot - self.ranges[layer].start,
        })
    }

    /// A Feature's dot, if its Layer is on show.
    pub fn dot(&self, feature: FeatureRef) -> Option<&Instance> {
        let range = self.ranges.get(feature.layer)?;
        self.dots.get(range.clone())?.get(feature.index)
    }

    /// Lends the gathered dots out, so the scene can draw them while the Layers
    /// themselves are being changed. Give them back with [`Self::restore_dots`].
    pub fn take_dots(&mut self) -> Vec<Instance> {
        std::mem::take(&mut self.dots)
    }

    pub fn restore_dots(&mut self, dots: Vec<Instance>) {
        self.dots = dots;
    }

    /// True if the Layer can show the moment `at`, given the real time now.
    pub fn covers(&self, layer: usize, at: JulianDate, real_now: JulianDate) -> bool {
        self.get(layer)
            .is_some_and(|layer| layer.time_coverage().covers(at, real_now))
    }

    /// Gathers the lines every Layer on show draws for the frame.
    pub fn lines(&self, selected: Option<FeatureRef>, at: JulianDate, out: &mut Vec<Segment>) {
        for index in 0..self.len() {
            let on_show = self
                .ranges
                .get(index)
                .is_some_and(|range| !range.is_empty());
            if let (true, Some(layer)) = (on_show, self.get(index)) {
                let selected = selected
                    .filter(|feature| feature.layer == index)
                    .map(|feature| feature.index);
                layer.lines(selected, at, out);
            }
        }
    }

    pub fn feature_name(&self, feature: FeatureRef) -> Option<&str> {
        self.get(feature.layer)?.feature_name(feature.index)
    }

    pub fn card(&self, feature: FeatureRef, at: JulianDate) -> Option<FeatureCard> {
        self.get(feature.layer)?.card(feature.index, at)
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use orion_core::layer::CardRow;

    use super::*;
    use crate::objects::Marks;
    use crate::orbit_layer::tests::snapshot_layer;

    /// A Layer of a few fixed places, for exercising `Layers` without real data.
    struct Places {
        names: Vec<&'static str>,
        coverage: TimeCoverage,
        on: bool,
        search: String,
        dots: Vec<Instance>,
        last_highlight: Highlight,
    }

    impl Places {
        fn new(names: &[&'static str], coverage: TimeCoverage) -> Box<Self> {
            Box::new(Self {
                names: names.to_vec(),
                coverage,
                on: true,
                search: String::new(),
                dots: Vec::new(),
                last_highlight: Highlight::default(),
            })
        }
    }

    impl Layer for Places {
        fn name(&self) -> &'static str {
            "Test places"
        }
        fn theme(&self) -> Theme {
            Theme::Places
        }
        fn time_coverage(&self) -> TimeCoverage {
            self.coverage
        }
        fn is_on(&self) -> bool {
            self.on
        }
        fn set_on(&mut self, on: bool) {
            self.on = on;
        }
        fn swatch(&self) -> [u8; 4] {
            [255; 4]
        }
        fn counts(&self) -> (usize, usize) {
            (self.found(usize::MAX).len(), self.names.len())
        }
        fn status(&self, _now: SystemTime) -> LayerStatus {
            LayerStatus {
                text: "Built in".to_owned(),
                refresh: None,
            }
        }
        fn set_search(&mut self, text: &str) {
            self.search = text.to_lowercase();
        }
        fn found(&self, limit: usize) -> Vec<(usize, String)> {
            self.names
                .iter()
                .enumerate()
                .filter(|(_, name)| name.to_lowercase().contains(&self.search))
                .take(limit)
                .map(|(index, name)| (index, name.to_string()))
                .collect()
        }
        fn update(&mut self, _at: JulianDate, highlight: Highlight) {
            self.last_highlight = highlight;
            self.dots = (0..self.names.len())
                .map(|index| {
                    let marks = Marks {
                        selected: highlight.selected == Some(index),
                        ..Marks::default()
                    };
                    Instance::new(Vec3::X * (1.1 + index as f32), [255; 4], marks)
                })
                .collect();
        }
        fn dots(&self) -> &[Instance] {
            &self.dots
        }
        fn lines(&self, _selected: Option<usize>, _at: JulianDate, _out: &mut Vec<Segment>) {}
        fn feature_name(&self, index: usize) -> Option<&str> {
            self.names.get(index).copied()
        }
        fn card(&self, index: usize, _at: JulianDate) -> Option<FeatureCard> {
            Some(FeatureCard {
                title: self.names.get(index)?.to_string(),
                sections: vec![vec![CardRow::new("Index", index.to_string())]],
                details: Vec::new(),
                notices: Vec::new(),
                source: "Test".to_owned(),
            })
        }
        fn stable_key(&self, index: usize) -> Option<u64> {
            (index < self.names.len()).then_some(1_000 + index as u64)
        }
        fn index_of(&self, key: u64) -> Option<usize> {
            key.checked_sub(1_000)
                .map(|index| index as usize)
                .filter(|index| *index < self.names.len())
        }
    }

    fn moment() -> JulianDate {
        JulianDate::from_utc(2026, 11, 24, 21, 0, 0.0)
    }

    fn layers() -> Layers {
        let mut layers = Layers::new(snapshot_layer());
        layers.add(Places::new(&["Alpha", "Beta"], TimeCoverage::AnyMoment));
        layers.add(Places::new(&["Gamma"], TimeCoverage::PresentOnly));
        layers
    }

    #[test]
    fn dots_from_every_layer_are_gathered_in_order() {
        let mut layers = layers();
        layers.update(moment(), moment(), None, None);

        let orbit = layers.orbit.dots().len();
        assert!(orbit > 29_000);
        assert_eq!(layers.dots().len(), orbit + 3);

        // A dot leads back to its Feature, and a Feature to its dot.
        let beta = FeatureRef { layer: 1, index: 1 };
        assert_eq!(layers.feature_at(orbit + 1), Some(beta));
        assert_eq!(
            layers.feature_at(orbit + 2),
            Some(FeatureRef { layer: 2, index: 0 })
        );
        assert_eq!(
            layers.feature_at(0),
            Some(FeatureRef { layer: 0, index: 0 })
        );
        assert_eq!(layers.feature_at(orbit + 3), None);
        assert_eq!(layers.dot(beta), Some(&layers.dots()[orbit + 1]));
        assert_eq!(layers.feature_name(beta), Some("Beta"));
        assert_eq!(layers.card(beta, moment()).unwrap().title, "Beta");
    }

    #[test]
    fn a_layer_that_is_off_shows_nothing_and_the_rest_close_up() {
        let mut layers = layers();
        layers.orbit.set_on(false);
        layers.update(moment(), moment(), None, None);

        assert_eq!(layers.dots().len(), 3);
        assert_eq!(
            layers.feature_at(0),
            Some(FeatureRef { layer: 1, index: 0 })
        );
        assert_eq!(layers.dot(FeatureRef { layer: 0, index: 0 }), None);
    }

    #[test]
    fn a_layer_drops_out_when_the_clock_leaves_its_time_coverage() {
        let mut layers = layers();
        let an_hour_ago = JulianDate(moment().0 - 1.0 / 24.0);
        layers.update(an_hour_ago, moment(), None, None);

        // The present-only Layer is gone; the other two can show any moment.
        assert_eq!(layers.dot(FeatureRef { layer: 2, index: 0 }), None);
        assert!(layers.dot(FeatureRef { layer: 1, index: 0 }).is_some());
        assert!(layers.dot(FeatureRef { layer: 0, index: 0 }).is_some());
    }

    #[test]
    fn a_selection_marks_only_its_own_layer() {
        let mut layers = layers();
        let beta = FeatureRef { layer: 1, index: 1 };
        layers.update(moment(), moment(), Some(beta), None);
        let selected = *layers.dot(beta).unwrap();

        layers.update(moment(), moment(), None, None);
        assert_ne!(*layers.dot(beta).unwrap(), selected);

        // The same index on another Layer is a different Feature and is left alone.
        layers.update(moment(), moment(), Some(beta), None);
        let orbit_dot = *layers.dot(FeatureRef { layer: 0, index: 1 }).unwrap();
        layers.update(moment(), moment(), None, None);
        assert_eq!(
            *layers.dot(FeatureRef { layer: 0, index: 1 }).unwrap(),
            orbit_dot
        );
    }

    #[test]
    fn dots_can_be_lent_out_and_given_back() {
        let mut layers = layers();
        layers.update(moment(), moment(), None, None);
        let beta = FeatureRef { layer: 1, index: 1 };
        let before = *layers.dot(beta).unwrap();

        let dots = layers.take_dots();
        // While they are out, a dot still maps to its Feature, but cannot be read.
        assert_eq!(
            layers.feature_at(dots.len() - 1),
            Some(FeatureRef { layer: 2, index: 0 })
        );
        assert_eq!(layers.dot(beta), None);

        layers.restore_dots(dots);
        assert_eq!(*layers.dot(beta).unwrap(), before);
    }

    #[test]
    fn search_and_counts_reach_every_layer() {
        let mut layers = layers();
        for index in 0..layers.len() {
            layers.get_mut(index).unwrap().set_search("ALPHA");
        }
        assert_eq!(layers.get(1).unwrap().counts(), (1, 2));
        assert_eq!(layers.get(1).unwrap().found(10), [(0, "Alpha".to_owned())]);
        assert_eq!(layers.get(2).unwrap().counts(), (0, 1));
        assert!(layers.covers(2, moment(), moment()));
        assert!(!layers.covers(2, JulianDate(moment().0 - 1.0), moment()));
        assert!(!layers.covers(9, moment(), moment()));
    }

    #[test]
    fn layers_are_reached_by_position() {
        let mut layers = layers();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers.get(0).unwrap().theme(), Theme::Orbit);
        assert_eq!(layers.get(1).unwrap().name(), "Test places");
        assert!(layers.get(3).is_none());

        layers.get_mut(1).unwrap().set_on(false);
        assert!(!layers.get(1).unwrap().is_on());
        assert_eq!(layers.get(1).unwrap().index_of(1_001), Some(1));
        assert_eq!(layers.get(1).unwrap().stable_key(1), Some(1_001));
    }
}
