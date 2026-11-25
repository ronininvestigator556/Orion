//! A Layer of Features at places on the ground, read from one listing: earthquakes,
//! fire detections, storms, alerts and whatever else is published as a list of places.
//!
//! Nothing is fetched while such a Layer is off. Once it is on, its listing is fetched
//! in the background and again each time the copy on hand grows stale.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime};

use glam::Vec3;
use orion_core::cluster;
use orion_core::feature::Feature;
use orion_core::layer::{FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::Cache;
use orion_data::listing::{Fetcher, Listing, ListingError, cached_listing, fetch_listing};

use crate::feed::Refresh;
use crate::layer::{ClusterMark, Highlight, Layer, LayerStatus, LayerView};
use crate::lines::Segment;
use crate::objects::{Instance, Marks};
use crate::picking::PickView;
use crate::reference::{on_globe, push_ground_line};
use crate::ui::format_duration;

/// The least time between two fetches of one listing, however often the user asks.
const MIN_INTERVAL: Duration = Duration::from_secs(60);
/// How long after a failed fetch the next is tried.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
/// How far apart, in logical points, Clusters are meant to sit on screen: at most
/// this, and at least half of it.
const CLUSTER_SPACING_POINTS: f64 = 48.0;
/// A Layer showing fewer Features than this is never gathered into Clusters: its
/// dots can be told apart from any distance.
const CLUSTER_ABOVE: usize = 200;
/// How opaque the lines of routes are.
const ROUTE_OPACITY: f32 = 0.85;
/// Colour of the outline drawn for the selected Feature.
const OUTLINE: [f32; 4] = [1.0, 1.0, 1.0, 0.9];
/// The most letters of a Feature's name used as the title of its card.
const TITLE_LETTERS: usize = 48;

/// One of the kinds a Layer divides its Features into, each with a colour and a
/// switch in the Layer's filter.
pub struct Class {
    pub label: &'static str,
    /// Colour as written for the screen (sRGB), red, green and blue.
    pub color: [u8; 3],
    /// Whether the filter lets the class through before the user has touched it.
    pub on: bool,
}

impl Class {
    /// A class that is shown until the user filters it out.
    pub const fn new(label: &'static str, color: [u8; 3]) -> Self {
        Self {
            label,
            color,
            on: true,
        }
    }

    /// A class that is hidden until the user asks for it, for kinds so numerous
    /// that they would bury the rest.
    pub const fn hidden(label: &'static str, color: [u8; 3]) -> Self {
        Self {
            label,
            color,
            on: false,
        }
    }
}

/// Which of a Layer's Features are on show at a given moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Show {
    /// Those that happened in the days before the moment, fading as they age.
    Recent { days: f64 },
    /// Those still in force at the moment.
    InForce,
    /// All of them, whatever the moment: places that stay where they are.
    Always,
}

/// Everything that makes one such Layer different from another.
pub struct Spec {
    pub name: &'static str,
    pub theme: Theme,
    pub listing: Listing,
    pub classes: &'static [Class],
    pub coverage: TimeCoverage,
    pub show: Show,
    /// How large a Feature's dot is, as a multiple of the usual size.
    pub dot_scale: fn(&Feature) -> f32,
    /// What a reader should know about the Layer as a whole.
    pub note: &'static str,
    /// Credit that must stay on screen while the Layer is on, where its Source asks
    /// for that; empty otherwise.
    pub attribution: &'static str,
    /// Whether Features too close together to tell apart are gathered into Clusters.
    /// Only for places that stay put: `show` must be [`Show::Always`].
    pub cluster: bool,
    /// Whether the Features are routes, drawn as their lines and not as dots.
    pub routes: bool,
}

/// Converts a screen colour to the linear colour the dot shader works in.
fn linear(color: [u8; 3]) -> [u8; 3] {
    color.map(|channel| ((f32::from(channel) / 255.0).powf(2.2) * 255.0).round() as u8)
}

pub struct FeatureLayer {
    spec: &'static Spec,
    fetcher: Arc<dyn Fetcher>,
    cache: Option<Cache>,
    on: bool,
    features: Vec<Feature>,
    /// Where each Feature is on the unit sphere.
    positions: Vec<Vec3>,
    /// Each Feature's name in lower case, for searching.
    names: Vec<String>,
    /// Which classes the filter lets through.
    class_on: Vec<bool>,
    search: String,
    /// Which Features pass the filter and the search.
    listed: Vec<bool>,
    listed_count: usize,
    dots: Vec<Instance>,
    /// Counts every change to which Features are listed, so dots that do not depend
    /// on the moment are only worked out again when something has changed.
    generation: u64,
    /// The highlight, generation and Cluster level the dots were last worked out for.
    dots_for: Option<(Highlight, u64, Option<u8>)>,
    /// The level of the grid that suits the view, or `None` when the view is close
    /// enough to tell Features apart.
    cluster_level: Option<u8>,
    clusters: Vec<ClusterMark>,
    /// Which Features are inside a Cluster and so have no dot of their own. Empty
    /// when nothing is clustered.
    clustered: Vec<bool>,
    /// The generation and level the Clusters were last worked out for.
    clusters_for: Option<(u64, Option<u8>)>,
    /// The lines of every route on show, for a Layer of routes.
    route_segments: Arc<Vec<Segment>>,
    /// Which Feature each of those segments belongs to.
    route_owners: Vec<u32>,
    /// The generation the routes were last worked out for.
    routes_for: Option<u64>,
    /// When the Features on hand were fetched.
    fetched_at: Option<SystemTime>,
    /// True once the cache has been looked in for an earlier copy.
    looked_in_cache: bool,
    in_flight: Option<Receiver<Result<Vec<Feature>, ListingError>>>,
    last_attempt: Option<SystemTime>,
    failure: Option<String>,
}

impl FeatureLayer {
    /// A Layer that is off and has fetched nothing.
    pub fn new(spec: &'static Spec, fetcher: Arc<dyn Fetcher>, cache: Option<Cache>) -> Self {
        Self {
            spec,
            fetcher,
            cache,
            on: false,
            features: Vec::new(),
            positions: Vec::new(),
            names: Vec::new(),
            class_on: spec.classes.iter().map(|class| class.on).collect(),
            search: String::new(),
            listed: Vec::new(),
            listed_count: 0,
            dots: Vec::new(),
            generation: 0,
            dots_for: None,
            cluster_level: None,
            clusters: Vec::new(),
            clustered: Vec::new(),
            clusters_for: None,
            route_segments: Arc::default(),
            route_owners: Vec::new(),
            routes_for: None,
            fetched_at: None,
            looked_in_cache: false,
            in_flight: None,
            last_attempt: None,
            failure: None,
        }
    }

    fn set_features(&mut self, features: Vec<Feature>, fetched_at: SystemTime) {
        self.positions = features
            .iter()
            .map(|feature| on_globe([feature.longitude_deg, feature.latitude_deg]))
            .collect();
        self.names = features
            .iter()
            .map(|feature| feature.name.to_lowercase())
            .collect();
        self.features = features;
        self.fetched_at = Some(fetched_at);
        self.dots.clear();
        self.refilter();
    }

    /// Works out afresh which Features pass the filter and the search.
    fn refilter(&mut self) {
        self.listed = self
            .features
            .iter()
            .zip(&self.names)
            .map(|(feature, name)| {
                self.class_on
                    .get(usize::from(feature.class))
                    .copied()
                    .unwrap_or(true)
                    && name.contains(&self.search)
            })
            .collect();
        self.listed_count = self.listed.iter().filter(|listed| **listed).count();
        self.generation += 1;
    }

    /// Takes in a fetch that has finished and starts one if the copy on hand is stale.
    /// Does nothing while the Layer is off.
    fn tick(&mut self, now: SystemTime) {
        if !self.on {
            return;
        }
        if !self.looked_in_cache {
            self.looked_in_cache = true;
            if let Some((features, fetched_at)) = self
                .cache
                .as_ref()
                .and_then(|cache| cached_listing(cache, &self.spec.listing))
            {
                self.set_features(features, fetched_at);
            }
        }
        if let Some(receiver) = &self.in_flight {
            match receiver.try_recv() {
                Ok(Ok(features)) => {
                    self.in_flight = None;
                    self.failure = None;
                    self.set_features(features, now);
                }
                Ok(Err(error)) => {
                    self.in_flight = None;
                    log::warn!("fetching {}: {error}", self.spec.name);
                    self.failure = Some(error.to_string());
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.in_flight = None;
                    self.failure = Some("the fetch stopped unexpectedly".to_owned());
                }
            }
        }
        let since = |earlier: Option<SystemTime>| {
            earlier.map(|earlier| now.duration_since(earlier).unwrap_or_default())
        };
        let stale = since(self.fetched_at).is_none_or(|age| age >= self.spec.listing.refresh_every);
        let wait = if self.failure.is_some() {
            RETRY_AFTER
        } else {
            MIN_INTERVAL
        };
        let may_try = since(self.last_attempt).is_none_or(|ago| ago >= wait);
        if stale && may_try {
            self.start_fetch(now);
        }
    }

    fn start_fetch(&mut self, now: SystemTime) {
        if self.in_flight.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let fetcher = Arc::clone(&self.fetcher);
        let cache = self.cache.clone();
        let listing = self.spec.listing;
        std::thread::spawn(move || {
            // The receiver is gone only if the app is shutting down.
            let _ = sender.send(fetch_listing(&*fetcher, cache.as_ref(), &listing));
        });
        self.in_flight = Some(receiver);
        self.last_attempt = Some(now);
    }

    /// How opaque a Feature's dot is at `at`, from 0 to 1, or `None` if the Feature
    /// is not on show then.
    fn opacity(&self, feature: &Feature, at: JulianDate) -> Option<f32> {
        match self.spec.show {
            Show::Recent { days } => {
                let age = at.0 - feature.time?.0;
                // Older ones fade, so the newest stand out.
                (0.0..=days)
                    .contains(&age)
                    .then(|| 1.0 - 0.6 * (age / days) as f32)
            }
            Show::InForce => feature
                .until
                .is_none_or(|until| at.0 < until.0)
                .then_some(1.0),
            Show::Always => Some(1.0),
        }
    }

    /// Gathers the Features on show into Clusters at the level that suits the view,
    /// if this Layer does that and has enough of them.
    fn recluster(&mut self) {
        let wanted = (self.generation, self.cluster_level);
        if self.clusters_for == Some(wanted) {
            return;
        }
        self.clusters_for = Some(wanted);
        self.clusters.clear();
        self.clustered.clear();
        let level = match self.cluster_level {
            Some(level)
                if self.spec.cluster
                    && self.spec.show == Show::Always
                    && self.listed_count >= CLUSTER_ABOVE =>
            {
                level
            }
            _ => return,
        };

        // For each square of the grid: the sum of its Features' positions and how many.
        let mut squares: HashMap<cluster::Cell, (Vec3, u32)> = HashMap::new();
        let cells: Vec<Option<cluster::Cell>> = self
            .features
            .iter()
            .zip(&self.listed)
            .map(|(feature, listed)| {
                listed.then(|| cluster::cell(feature.latitude_deg, feature.longitude_deg, level))
            })
            .collect();
        for (cell, position) in cells.iter().zip(&self.positions) {
            if let Some(cell) = cell {
                let square = squares.entry(*cell).or_insert((Vec3::ZERO, 0));
                square.0 += *position;
                square.1 += 1;
            }
        }
        let [r, g, b, _] = self.swatch();
        self.clusters = squares
            .values()
            .filter(|(_, count)| *count >= 2)
            .map(|(sum, count)| ClusterMark {
                position: sum.normalize_or_zero(),
                count: *count,
                color: [r, g, b],
            })
            .collect();
        // A stable order, so the marks do not shuffle from one frame to the next.
        self.clusters.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.position.x.total_cmp(&b.position.x))
        });
        self.clustered = cells
            .iter()
            .map(|cell| cell.is_some_and(|cell| squares[&cell].1 >= 2))
            .collect();
    }

    /// Lays out the lines of every route on show, for a Layer of routes.
    fn rebuild_routes(&mut self) {
        if !self.spec.routes || self.routes_for == Some(self.generation) {
            return;
        }
        self.routes_for = Some(self.generation);
        let colors: Vec<[f32; 4]> = self
            .spec
            .classes
            .iter()
            .map(|class| {
                let [r, g, b] = linear(class.color).map(|channel| f32::from(channel) / 255.0);
                [r, g, b, ROUTE_OPACITY]
            })
            .collect();
        let mut segments = Vec::new();
        self.route_owners.clear();
        for (index, feature) in self.features.iter().enumerate() {
            if !self.listed[index] {
                continue;
            }
            let color = colors.get(usize::from(feature.class)).copied().unwrap_or([
                1.0,
                1.0,
                1.0,
                ROUTE_OPACITY,
            ]);
            for line in &feature.shape {
                push_ground_line(&mut segments, line, color);
            }
            self.route_owners.resize(segments.len(), index as u32);
        }
        self.route_segments = Arc::new(segments);
    }

    fn rebuild_dots(&mut self, at: JulianDate, highlight: Highlight) {
        self.rebuild_routes();
        self.recluster();
        // Places that stay put look the same from one frame to the next unless the
        // filter, the search, the data, the highlight or the Clusters have changed.
        let wanted = (highlight, self.generation, self.cluster_level);
        if self.spec.show == Show::Always
            && self.dots_for == Some(wanted)
            && self.dots.len() == self.features.len()
        {
            return;
        }
        self.dots_for = Some(wanted);
        let colors: Vec<[u8; 3]> = self
            .spec
            .classes
            .iter()
            .map(|class| linear(class.color))
            .collect();
        let dots = self
            .features
            .iter()
            .enumerate()
            .map(|(index, feature)| {
                let Some(opacity) = self.opacity(feature, at).filter(|_| self.listed[index]) else {
                    return Instance::HIDDEN;
                };
                // Inside a Cluster a Feature has no dot of its own, unless it is the
                // one selected, which must stay findable.
                // A route is drawn as its line; only the selected one has a dot as well,
                // to carry its name.
                let in_cluster = self.clustered.get(index).copied().unwrap_or(false);
                if (in_cluster || self.spec.routes) && highlight.selected != Some(index) {
                    return Instance::HIDDEN;
                }
                let [r, g, b] = colors
                    .get(usize::from(feature.class))
                    .copied()
                    .unwrap_or([255; 3]);
                let marks = Marks {
                    selected: highlight.selected == Some(index),
                    hovered: highlight.hovered == Some(index),
                    stale: false,
                };
                Instance::new(
                    self.positions[index],
                    [r, g, b, (opacity * 255.0) as u8],
                    marks,
                )
                .scaled((self.spec.dot_scale)(feature))
            })
            .collect();
        self.dots = dots;
    }
}

impl Layer for FeatureLayer {
    fn name(&self) -> &'static str {
        self.spec.name
    }

    fn theme(&self) -> Theme {
        self.spec.theme
    }

    fn time_coverage(&self) -> TimeCoverage {
        self.spec.coverage
    }

    fn is_on(&self) -> bool {
        self.on
    }

    fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.dots.clear();
            self.dots_for = None;
            self.clusters.clear();
            self.clustered.clear();
            self.clusters_for = None;
            self.route_segments = Arc::default();
            self.route_owners.clear();
            self.routes_for = None;
        }
    }

    fn swatch(&self) -> [u8; 4] {
        let [r, g, b] = self
            .spec
            .classes
            .first()
            .map_or([255; 3], |class| linear(class.color));
        [r, g, b, 255]
    }

    fn counts(&self) -> (usize, usize) {
        (self.listed_count, self.features.len())
    }

    fn status(&self, now: SystemTime) -> LayerStatus {
        let source = self.spec.listing.source;
        let age = self
            .fetched_at
            .map(|at| format_duration(now.duration_since(at).unwrap_or_default()));
        let mut text = match (&self.failure, &age) {
            (Some(reason), Some(age)) => format!(
                "Could not reach {source}: {reason}. Showing what was fetched {age} ago; trying again shortly."
            ),
            (Some(reason), None) => {
                format!("Could not reach {source}: {reason}. Trying again shortly.")
            }
            (None, Some(age)) => format!("From {source}, fetched {age} ago."),
            (None, None) => format!("Fetching from {source}."),
        };
        if !self.spec.note.is_empty() {
            text.push(' ');
            text.push_str(self.spec.note);
        }
        let since_attempt = self
            .last_attempt
            .map(|at| now.duration_since(at).unwrap_or_default());
        let refresh = match since_attempt {
            _ if self.in_flight.is_some() => Refresh::InProgress,
            Some(ago) if ago < MIN_INTERVAL => Refresh::Locked(MIN_INTERVAL - ago),
            _ => Refresh::Ready,
        };
        LayerStatus {
            text,
            refresh: Some(refresh),
        }
    }

    fn request_refresh(&mut self, now: SystemTime) {
        let recently = self
            .last_attempt
            .is_some_and(|at| now.duration_since(at).unwrap_or_default() < MIN_INTERVAL);
        if self.on && !recently {
            self.failure = None;
            self.start_fetch(now);
        }
    }

    fn set_search(&mut self, text: &str) {
        let text = text.trim().to_lowercase();
        if text != self.search {
            self.search = text;
            self.refilter();
        }
    }

    fn found(&self, limit: usize) -> Vec<(usize, String)> {
        self.features
            .iter()
            .enumerate()
            .filter(|(index, _)| self.listed[*index])
            .take(limit)
            .map(|(index, feature)| (index, feature.name.clone()))
            .collect()
    }

    fn filter_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        for (class, on) in self.spec.classes.iter().zip(&mut self.class_on) {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(on, "").changed();
                let [r, g, b] = class.color;
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(swatch, 5.0, egui::Color32::from_rgb(r, g, b));
                ui.label(class.label);
            });
        }
        if changed {
            self.refilter();
        }
    }

    fn update(&mut self, at: JulianDate, highlight: Highlight) {
        self.tick(SystemTime::now());
        self.rebuild_dots(at, highlight);
    }

    fn dots(&self) -> &[Instance] {
        &self.dots
    }

    fn lines(&self, selected: Option<usize>, _at: JulianDate, out: &mut Vec<Segment>) {
        let Some(feature) = selected.and_then(|index| self.features.get(index)) else {
            return;
        };
        for line in &feature.shape {
            push_ground_line(out, line, OUTLINE);
        }
    }

    fn feature_name(&self, index: usize) -> Option<&str> {
        self.features
            .get(index)
            .map(|feature| feature.name.as_str())
    }

    fn card(&self, index: usize, _at: JulianDate) -> Option<FeatureCard> {
        let feature = self.features.get(index)?;
        let mut title: String = feature.name.chars().take(TITLE_LETTERS).collect();
        if title.len() < feature.name.len() {
            title.push('…');
        }
        let mut details = feature.details.clone();
        if !self.spec.note.is_empty() {
            details.push(self.spec.note.to_owned());
        }
        Some(FeatureCard {
            title,
            sections: vec![feature.card_rows()],
            details,
            notices: Vec::new(),
            source: self.spec.listing.source.to_owned(),
        })
    }

    fn stable_key(&self, index: usize) -> Option<u64> {
        self.features.get(index).map(|feature| feature.key)
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        self.features.iter().position(|feature| feature.key == key)
    }

    fn is_loading(&self) -> bool {
        self.on && self.fetched_at.is_none() && self.failure.is_none()
    }

    fn attribution(&self) -> Option<&'static str> {
        (!self.spec.attribution.is_empty()).then_some(self.spec.attribution)
    }

    fn set_view(&mut self, view: &LayerView) {
        self.cluster_level = cluster::level_for(view.degrees_per_point, CLUSTER_SPACING_POINTS);
    }

    fn clusters(&self) -> &[ClusterMark] {
        &self.clusters
    }

    fn position(&self, index: usize) -> Option<Vec3> {
        self.positions.get(index).copied()
    }

    fn routes(&self) -> Option<(u64, Arc<Vec<Segment>>)> {
        self.spec.routes.then(|| {
            (
                self.routes_for.unwrap_or(0),
                Arc::clone(&self.route_segments),
            )
        })
    }

    fn pick_route(&self, view: &PickView, cursor: glam::Vec2, reach: f32) -> Option<usize> {
        if !self.spec.routes {
            return None;
        }
        self.route_segments
            .iter()
            .zip(&self.route_owners)
            .filter_map(|(segment, owner)| {
                let (start, end) = segment.ends();
                // A piece with an end out of sight is passed over: the pieces are
                // short, so its neighbours stand in for it.
                let (start, end) = (view.project(start)?, view.project(end)?);
                let along = end - start;
                let t = if along.length_squared() == 0.0 {
                    0.0
                } else {
                    ((cursor - start).dot(along) / along.length_squared()).clamp(0.0, 1.0)
                };
                let distance = cursor.distance(start + along * t);
                (distance <= reach).then_some((distance, *owner as usize))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, owner)| owner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use orion_data::FetchError;
    use orion_data::listing::ParseError;

    use super::*;

    /// Answers every fetch with the next of a list of prepared answers, repeating the last.
    struct Stub {
        answers: Mutex<Vec<Result<String, FetchError>>>,
        fetches: AtomicUsize,
    }

    impl Stub {
        fn answering(answers: Vec<Result<&str, FetchError>>) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(
                    answers
                        .into_iter()
                        .map(|answer| answer.map(str::to_owned))
                        .collect(),
                ),
                fetches: AtomicUsize::new(0),
            })
        }

        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    impl Fetcher for Stub {
        fn get(&self, _url: &str, _accept: Option<&str>) -> Result<String, FetchError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            let mut answers = self.answers.lock().unwrap();
            if answers.len() > 1 {
                answers.remove(0)
            } else {
                answers[0].clone()
            }
        }
    }

    /// Lines of `name,class,days after the start,latitude,longitude`.
    fn parse(text: &str) -> Result<Vec<Feature>, ParseError> {
        text.lines()
            .map(|line| {
                let fields: Vec<&str> = line.split(',').collect();
                let [name, class, day, latitude, longitude] = fields[..] else {
                    return Err(ParseError(format!("bad line {line:?}")));
                };
                let mut feature = Feature::at(
                    name,
                    name,
                    latitude.parse().unwrap(),
                    longitude.parse().unwrap(),
                );
                feature.class = class.parse().unwrap();
                feature.time = Some(JulianDate(start().0 + day.parse::<f64>().unwrap()));
                feature.row("Name", name);
                feature.shape = vec![vec![[0.0, 0.0], [1.0, 0.0]]];
                Ok(feature)
            })
            .collect()
    }

    fn start() -> JulianDate {
        JulianDate::from_utc(2026, 11, 20, 0, 0, 0.0)
    }

    static SPEC: Spec = Spec {
        name: "Test events",
        theme: Theme::Weather,
        listing: Listing {
            source: "Test Source",
            url: "http://example.invalid/events",
            accept: None,
            cache_name: "test_events.csv",
            parse,
            refresh_every: Duration::from_secs(600),
        },
        classes: &[
            Class::new("Big", [255, 0, 0]),
            Class::new("Small", [255, 255, 0]),
        ],
        coverage: TimeCoverage::PresentAndPast { days: 7.0 },
        show: Show::Recent { days: 2.0 },
        dot_scale: |_| 2.0,
        note: "Only a test.",
        attribution: "",
        cluster: false,
        routes: false,
    };

    /// The test events again as routes: each has a line a degree long at the equator.
    static ROUTES: Spec = Spec {
        name: "Test routes",
        theme: Theme::Energy,
        listing: SPEC.listing,
        classes: &[
            Class::new("Big", [255, 0, 0]),
            Class::new("Small", [255, 255, 0]),
        ],
        coverage: TimeCoverage::AnyMoment,
        show: Show::Always,
        dot_scale: |_| 1.0,
        note: "",
        attribution: "",
        cluster: false,
        routes: true,
    };

    #[test]
    fn routes_are_drawn_as_lines_and_picked_by_them() {
        use crate::camera::OrbitCamera;
        use glam::Vec2;

        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = FeatureLayer::new(&ROUTES, stub as Arc<dyn Fetcher>, None);
        assert!(layer.routes().is_some());
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());
        layer.rebuild_dots(start(), Highlight::default());

        // Three Features, each a line of four pieces, and no dots.
        let (generation, segments) = layer.routes().unwrap();
        assert_eq!(segments.len(), 12);
        assert!(shown(&layer).is_empty());

        // Nothing changes until the filter does: then only the first Feature's line is left.
        layer.rebuild_dots(start(), Highlight::default());
        assert_eq!(layer.routes().unwrap().0, generation);
        layer.class_on[1] = false;
        layer.refilter();
        layer.rebuild_dots(start(), Highlight::default());
        let (later, segments) = layer.routes().unwrap();
        assert_ne!(later, generation);
        assert_eq!(segments.len(), 4);

        // The selected route has a dot as well, to carry its name.
        let highlight = Highlight {
            selected: Some(0),
            hovered: None,
        };
        layer.rebuild_dots(start(), highlight);
        assert_eq!(shown(&layer), ["Alpha"]);

        // Looking straight down on the line, a click on it finds its Feature, and a
        // click well away from it finds nothing.
        let mut camera = OrbitCamera::default();
        camera.face(on_globe([0.5, 0.0]));
        camera.set_altitude(0.05);
        let viewport = Vec2::new(1000.0, 1000.0);
        let view = PickView {
            view_projection: camera.view_projection(1.0),
            camera_position: camera.position(),
            viewport,
        };
        assert_eq!(layer.pick_route(&view, viewport / 2.0, 6.0), Some(0));
        assert_eq!(layer.pick_route(&view, Vec2::new(500.0, 300.0), 6.0), None);
    }

    /// The same events as places that stay put, with the second class filtered out
    /// until asked for.
    static PLACES: Spec = Spec {
        name: "Test places",
        theme: Theme::Places,
        listing: SPEC.listing,
        classes: &[
            Class::new("Big", [255, 0, 0]),
            Class::hidden("Small", [255, 255, 0]),
        ],
        coverage: TimeCoverage::AnyMoment,
        show: Show::Always,
        dot_scale: |_| 1.0,
        note: "",
        attribution: "Test credit",
        cluster: true,
        routes: false,
    };

    /// Three hundred places: a tight knot of 250 near Paris, 49 strung along the
    /// equator a degree apart, and one alone in the far south.
    fn many_places() -> String {
        let mut lines = Vec::new();
        for index in 0..250 {
            let step = f64::from(index) * 0.0004;
            lines.push(format!("Knot {index},0,0,{},{}", 48.8 + step, 2.3 + step));
        }
        for index in 0..49 {
            lines.push(format!("String {index},0,0,0.5,{}", f64::from(index) + 0.5));
        }
        lines.push("Alone,0,0,-60.5,100.5".to_owned());
        lines.join("\n")
    }

    fn clustering_layer(text: &'static str) -> FeatureLayer {
        let stub = Stub::answering(vec![Ok(text)]);
        let mut layer = FeatureLayer::new(&PLACES, stub as Arc<dyn Fetcher>, None);
        layer.class_on[1] = true;
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());
        layer
    }

    fn look_from(layer: &mut FeatureLayer, degrees_per_point: f64, highlight: Highlight) {
        layer.set_view(&LayerView { degrees_per_point });
        layer.rebuild_dots(start(), highlight);
    }

    #[test]
    fn from_far_away_crowded_places_become_clusters() {
        let text: &'static str = Box::leak(many_places().into_boxed_str());
        let mut layer = clustering_layer(text);
        assert_eq!(layer.counts(), (300, 300));

        // From the default distance, squares are 2.8 degrees across: the knot is one
        // Cluster, and the string falls into a Cluster every two or three places.
        look_from(&mut layer, 0.065, Highlight::default());
        let clusters = layer.clusters().to_vec();
        assert_eq!(clusters[0].count, 250);
        assert!(clusters[0].position.dot(on_globe([2.35, 48.85])) > 0.999_99);
        assert!(clusters.len() > 10, "{}", clusters.len());
        let in_clusters: u32 = clusters.iter().map(|cluster| cluster.count).sum();
        let dots = layer.dots().iter().filter(|dot| !dot.is_hidden()).count();
        // Every place is either in a Cluster or a dot of its own, never both.
        assert_eq!(in_clusters as usize + dots, 300);
        assert!(shown(&layer).contains(&"Alone"));
        assert!(!shown(&layer).contains(&"Knot 0"));
    }

    #[test]
    fn clusters_break_up_as_the_view_comes_closer() {
        let text: &'static str = Box::leak(many_places().into_boxed_str());
        let mut layer = clustering_layer(text);
        look_from(&mut layer, 0.065, Highlight::default());
        let far = layer.clusters().len();

        // Closer, the string along the equator comes apart into dots.
        look_from(&mut layer, 0.005, Highlight::default());
        assert!(layer.clusters().len() < far);
        assert!(shown(&layer).contains(&"String 7"));
        assert!(!shown(&layer).contains(&"Knot 0"));

        // Close enough to tell places apart, there are no Clusters at all.
        look_from(&mut layer, 0.000_1, Highlight::default());
        assert!(layer.clusters().is_empty());
        assert_eq!(shown(&layer).len(), 300);
    }

    #[test]
    fn the_selected_feature_keeps_its_dot_inside_a_cluster() {
        let text: &'static str = Box::leak(many_places().into_boxed_str());
        let mut layer = clustering_layer(text);
        let highlight = Highlight {
            selected: Some(3),
            hovered: None,
        };
        look_from(&mut layer, 0.065, highlight);
        assert!(shown(&layer).contains(&"Knot 3"));
        assert!(!shown(&layer).contains(&"Knot 4"));
        assert_eq!(layer.clusters()[0].count, 250);
        assert_eq!(layer.position(3), Some(layer.positions[3]));
    }

    #[test]
    fn a_layer_with_few_features_on_show_is_never_clustered() {
        let text: &'static str = Box::leak(many_places().into_boxed_str());
        let mut layer = clustering_layer(text);
        // The search leaves the 49 along the equator, under the threshold.
        layer.set_search("string");
        look_from(&mut layer, 0.065, Highlight::default());
        assert!(layer.clusters().is_empty());
        assert_eq!(shown(&layer).len(), 49);

        // And the three test events never cluster, however far away.
        let mut small = clustering_layer(EVENTS);
        look_from(&mut small, 1.0, Highlight::default());
        assert!(small.clusters().is_empty());
        assert_eq!(shown(&small).len(), 3);
    }

    #[test]
    fn places_are_shown_at_any_moment_and_some_classes_start_hidden() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = FeatureLayer::new(&PLACES, Arc::clone(&stub) as Arc<dyn Fetcher>, None);
        assert_eq!(layer.attribution(), Some("Test credit"));
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());
        assert_eq!(layer.counts(), (1, 3));

        // Years before any of them "happened", the one unfiltered place is still there.
        layer.rebuild_dots(JulianDate(start().0 - 5_000.0), Highlight::default());
        assert_eq!(shown(&layer), ["Alpha"]);

        layer.class_on[1] = true;
        layer.refilter();
        layer.rebuild_dots(start(), Highlight::default());
        assert_eq!(shown(&layer), ["Alpha", "Bravo", "Charlie"]);

        // Selecting one changes its dot, and only then are the dots worked out again.
        let before = layer.dots().to_vec();
        layer.rebuild_dots(start(), Highlight::default());
        assert_eq!(layer.dots(), before);
        let highlight = Highlight {
            selected: Some(2),
            hovered: None,
        };
        layer.rebuild_dots(start(), highlight);
        assert_ne!(layer.dots()[2], before[2]);
        assert_eq!(layer.dots()[..2], before[..2]);
    }

    const EVENTS: &str = "Alpha,0,1.0,10,20\nBravo,1,2.5,-5,100\nCharlie,1,3.0,40,-70";

    fn layer(stub: &Arc<Stub>, cache: Option<Cache>) -> FeatureLayer {
        FeatureLayer::new(&SPEC, Arc::clone(stub) as Arc<dyn Fetcher>, cache)
    }

    /// Runs the Layer until the fetch in flight has been taken in.
    fn settle(layer: &mut FeatureLayer, now: SystemTime) {
        for _ in 0..500 {
            layer.tick(now);
            if layer.in_flight.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the fetch never finished");
    }

    fn shown(layer: &FeatureLayer) -> Vec<&str> {
        layer
            .dots
            .iter()
            .zip(&layer.features)
            .filter(|(dot, _)| !dot.is_hidden())
            .map(|(_, feature)| feature.name.as_str())
            .collect()
    }

    #[test]
    fn nothing_is_fetched_until_the_layer_is_switched_on() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        let now = SystemTime::now();
        layer.tick(now);
        layer.update(start(), Highlight::default());
        assert_eq!(stub.fetches(), 0);
        assert_eq!(layer.counts(), (0, 0));
        assert!(!layer.is_loading());

        layer.set_on(true);
        assert!(layer.is_loading());
        settle(&mut layer, now);
        assert!(!layer.is_loading());
        assert_eq!(stub.fetches(), 1);
        assert_eq!(layer.counts(), (3, 3));
        assert!(
            layer
                .status(now)
                .text
                .starts_with("From Test Source, fetched 0 min ago.")
        );
        assert!(layer.status(now).text.ends_with("Only a test."));
    }

    #[test]
    fn a_fresh_copy_is_not_fetched_again_until_it_is_stale() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        let now = SystemTime::now();
        settle(&mut layer, now);

        layer.tick(now + Duration::from_secs(599));
        assert_eq!(stub.fetches(), 1);
        settle(&mut layer, now + Duration::from_secs(601));
        assert_eq!(stub.fetches(), 2);
    }

    #[test]
    fn only_what_happened_in_the_days_before_the_moment_is_on_show() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());
        let at = |days: f64| JulianDate(start().0 + days);

        layer.rebuild_dots(at(0.5), Highlight::default());
        assert!(shown(&layer).is_empty());
        layer.rebuild_dots(at(2.6), Highlight::default());
        assert_eq!(shown(&layer), ["Alpha", "Bravo"]);
        layer.rebuild_dots(at(3.5), Highlight::default());
        assert_eq!(shown(&layer), ["Bravo", "Charlie"]);
        // Hidden Features keep their place, so indexes still line up.
        assert_eq!(layer.dots().len(), 3);
        assert!(layer.dots()[0].is_hidden());
    }

    #[test]
    fn the_filter_and_the_search_narrow_what_is_listed_and_shown() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());

        layer.class_on[1] = false;
        layer.refilter();
        assert_eq!(layer.counts(), (1, 3));
        layer.rebuild_dots(JulianDate(start().0 + 2.6), Highlight::default());
        assert_eq!(shown(&layer), ["Alpha"]);

        layer.class_on[1] = true;
        layer.refilter();
        layer.set_search("  CHAR ");
        assert_eq!(layer.found(10), [(2, "Charlie".to_owned())]);
        layer.set_search("");
        assert_eq!(layer.found(2).len(), 2);
    }

    #[test]
    fn a_failed_fetch_keeps_what_is_on_hand_and_is_tried_again_later() {
        let stub = Stub::answering(vec![
            Ok(EVENTS),
            Err(FetchError::Refused(503)),
            Ok("Delta,0,1.0,0,0"),
        ]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        let now = SystemTime::now();
        settle(&mut layer, now);

        let later = now + Duration::from_secs(700);
        settle(&mut layer, later);
        assert_eq!(stub.fetches(), 2);
        assert_eq!(layer.counts(), (3, 3));
        let status = layer.status(later).text;
        assert!(
            status.starts_with("Could not reach Test Source"),
            "{status}"
        );
        assert!(
            status.contains("Showing what was fetched 12 min ago"),
            "{status}"
        );

        // Not straight away, but once a few minutes have passed.
        layer.tick(later + Duration::from_secs(200));
        assert_eq!(stub.fetches(), 2);
        settle(&mut layer, later + Duration::from_secs(301));
        assert_eq!(stub.fetches(), 3);
        assert_eq!(layer.counts(), (1, 1));
        assert!(layer.status(later).text.starts_with("From Test Source"));
    }

    #[test]
    fn an_earlier_copy_is_shown_at_once_and_spared_a_fetch_while_fresh() {
        let folder = tempfile::tempdir().unwrap();
        let cache = Cache::at(folder.path());
        cache.store_layer_text("test_events.csv", EVENTS).unwrap();
        let stub = Stub::answering(vec![Ok("Delta,0,1.0,0,0")]);
        let mut layer = layer(&stub, Some(cache));
        layer.set_on(true);
        layer.tick(SystemTime::now());
        assert_eq!(layer.counts(), (3, 3));
        assert_eq!(stub.fetches(), 0);
    }

    #[test]
    fn asking_for_a_refresh_is_limited_to_once_a_minute() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        let now = SystemTime::now();
        settle(&mut layer, now);
        assert_eq!(
            layer.status(now + Duration::from_secs(20)).refresh,
            Some(Refresh::Locked(Duration::from_secs(40)))
        );
        layer.request_refresh(now + Duration::from_secs(20));
        assert_eq!(stub.fetches(), 1);

        let later = now + Duration::from_secs(61);
        assert_eq!(layer.status(later).refresh, Some(Refresh::Ready));
        layer.request_refresh(later);
        assert_eq!(layer.status(later).refresh, Some(Refresh::InProgress));
        settle(&mut layer, later);
        assert_eq!(stub.fetches(), 2);
    }

    #[test]
    fn a_selected_feature_shows_its_card_and_outline_and_keeps_its_key() {
        let stub = Stub::answering(vec![Ok(EVENTS)]);
        let mut layer = layer(&stub, None);
        layer.set_on(true);
        settle(&mut layer, SystemTime::now());

        let card = layer.card(1, start()).unwrap();
        assert_eq!(card.title, "Bravo");
        assert_eq!(card.source, "Test Source");
        assert_eq!(card.details, ["Only a test."]);
        assert!(card.notices.is_empty());
        assert_eq!(card.sections[0][0].value, "Bravo");

        let mut segments = Vec::new();
        layer.lines(Some(1), start(), &mut segments);
        assert_eq!(segments.len(), 4);
        layer.lines(None, start(), &mut segments);
        assert_eq!(segments.len(), 4);

        let key = layer.stable_key(1).unwrap();
        assert_eq!(layer.index_of(key), Some(1));
        assert_eq!(layer.index_of(key + 1), None);
        assert_eq!(layer.feature_name(1), Some("Bravo"));
    }
}
