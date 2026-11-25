//! Layers that are pictures laid over the Base Map: the day's clouds, the view from
//! a weather satellite, how hard it rained.
//!
//! Such a Layer has no Features. It follows the Simulation Clock: move the clock back
//! and the picture is the one from then, as far back as the Source keeps them.

use std::time::{Duration, Instant, SystemTime};

use orion_core::layer::{FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::tiles::{self, TileLayer};

use crate::basemap::Picture;
use crate::layer::{Highlight, Layer, LayerStatus};
use crate::lines::Segment;
use crate::objects::Instance;

/// Who publishes every picture here, as shown to the user.
const SOURCE: &str = "NASA's Global Imagery Browse Services";
const MINUTE: f64 = 1.0 / 1_440.0;
/// How long the moment wanted must stay the same before its picture is fetched. With
/// the Simulation Clock running fast the moment changes many times a second, and
/// asking the Source for each one would be a flood of requests for pictures nobody sees.
const SETTLE: Duration = Duration::from_millis(1200);

/// How often a picture is made, and how long after the fact it is published.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// One a day, published this many days later.
    Daily { lag_days: f64 },
    /// One every ten minutes, published this many minutes later.
    TenMinutes { lag_minutes: f64 },
}

/// Everything that makes one picture Layer different from another.
pub struct PictureSpec {
    pub name: &'static str,
    pub tiles: TileLayer,
    pub step: Step,
    /// How far back the Source keeps the pictures, in days.
    pub kept_days: f64,
    pub opacity: f32,
    /// Whether black in the picture means nothing was seen there.
    pub black_is_empty: bool,
    /// What a reader should know about the picture.
    pub note: &'static str,
    /// The colour that stands for the Layer in the sidebar, as written for the screen.
    pub swatch: [u8; 3],
}

/// The moment whose picture is shown when the Simulation Clock is at `at`: the clock's
/// own moment, or the latest one published if the clock is ahead of that, rounded
/// down to when a picture was made. Returned with the way the Source writes it.
pub fn picture_moment(step: Step, at: JulianDate, real_now: JulianDate) -> (JulianDate, String) {
    match step {
        Step::Daily { lag_days } => {
            let moment = JulianDate(at.0.min(real_now.0 - lag_days));
            let day = moment.to_utc();
            let start = JulianDate::from_utc(day.year, day.month, day.day, 0, 0, 0.0);
            (
                start,
                format!("{:04}-{:02}-{:02}", day.year, day.month, day.day),
            )
        }
        Step::TenMinutes { lag_minutes } => {
            // A second's grace, so a moment on the mark is not rounded down past it.
            let moment = JulianDate(at.0.min(real_now.0 - lag_minutes * MINUTE) + MINUTE / 60.0);
            let time = moment.to_utc();
            let minute = time.minute / 10 * 10;
            let start =
                JulianDate::from_utc(time.year, time.month, time.day, time.hour, minute, 0.0);
            (
                start,
                format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
                    time.year, time.month, time.day, time.hour, minute
                ),
            )
        }
    }
}

pub struct PictureLayer {
    spec: &'static PictureSpec,
    on: bool,
    /// The moment of the picture on show, as the Source writes it.
    showing: Option<String>,
    /// The moment the Simulation Clock calls for, and since when it has.
    wanted: Option<(String, Instant)>,
}

impl PictureLayer {
    /// A Layer that is off.
    pub fn new(spec: &'static PictureSpec) -> Self {
        Self {
            spec,
            on: false,
            showing: None,
            wanted: None,
        }
    }

    /// Notes the moment the Simulation Clock calls for, and shows its picture once
    /// the clock has called for it long enough. The first picture is shown at once.
    fn show(&mut self, at: JulianDate, real_now: JulianDate, now: Instant) {
        let (_, written) = picture_moment(self.spec.step, at, real_now);
        if self.showing.is_none() {
            self.showing = Some(written.clone());
        }
        match &self.wanted {
            Some((wanted, since)) if *wanted == written => {
                if now.duration_since(*since) >= SETTLE {
                    self.showing = Some(written);
                }
            }
            _ => self.wanted = Some((written, now)),
        }
    }
}

fn real_now() -> JulianDate {
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    JulianDate::from_unix_seconds(seconds)
}

impl Layer for PictureLayer {
    fn name(&self) -> &'static str {
        self.spec.name
    }

    fn theme(&self) -> Theme {
        Theme::Weather
    }

    fn time_coverage(&self) -> TimeCoverage {
        TimeCoverage::PresentAndPast {
            days: self.spec.kept_days,
        }
    }

    fn is_on(&self) -> bool {
        self.on
    }

    fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.showing = None;
            self.wanted = None;
        }
    }

    fn swatch(&self) -> [u8; 4] {
        let [r, g, b] = self
            .spec
            .swatch
            .map(|channel| ((f32::from(channel) / 255.0).powf(2.2) * 255.0).round() as u8);
        [r, g, b, 255]
    }

    fn has_features(&self) -> bool {
        false
    }

    fn counts(&self) -> (usize, usize) {
        (0, 0)
    }

    fn status(&self, _now: SystemTime) -> LayerStatus {
        let mut text = match &self.showing {
            Some(moment) => {
                let moment = moment.replace('T', " ").replace(":00Z", " UTC");
                format!("From {SOURCE}. Showing the picture for {moment}.")
            }
            None => format!("From {SOURCE}."),
        };
        text.push(' ');
        text.push_str(self.spec.note);
        LayerStatus {
            text,
            refresh: None,
        }
    }

    fn set_search(&mut self, _text: &str) {}

    fn found(&self, _limit: usize) -> Vec<(usize, String)> {
        Vec::new()
    }

    fn update(&mut self, at: JulianDate, _highlight: Highlight) {
        self.show(at, real_now(), Instant::now());
    }

    fn dots(&self) -> &[Instance] {
        &[]
    }

    fn lines(&self, _selected: Option<usize>, _at: JulianDate, _out: &mut Vec<Segment>) {}

    fn feature_name(&self, _index: usize) -> Option<&str> {
        None
    }

    fn card(&self, _index: usize, _at: JulianDate) -> Option<FeatureCard> {
        None
    }

    fn stable_key(&self, _index: usize) -> Option<u64> {
        None
    }

    fn index_of(&self, _key: u64) -> Option<usize> {
        None
    }

    fn picture(&self) -> Option<Picture> {
        let moment = self.showing.as_ref()?;
        Some(Picture::Overlay {
            tiles: self.spec.tiles.at(moment.as_str()),
            opacity: self.spec.opacity,
            black_is_empty: self.spec.black_is_empty,
        })
    }
}

const GEOSTATIONARY_NOTE: &str = "A new picture every ten minutes, about three quarters of an \
     hour after it is taken. It covers the side of Earth its satellite faces.";

/// The day's view of the whole Earth, clouds and all.
pub static DAILY_TRUE_COLOUR: PictureSpec = PictureSpec {
    name: "Clouds, day by day",
    tiles: tiles::DAILY_TRUE_COLOUR,
    step: Step::Daily { lag_days: 1.0 },
    kept_days: 14.0,
    opacity: 0.92,
    black_is_empty: true,
    note: "Each place as a satellite saw it in daylight on that day. The latest whole \
           day is yesterday's.",
    swatch: [236, 239, 241],
};

pub static GOES_EAST: PictureSpec = PictureSpec {
    name: "Weather satellite: Americas",
    tiles: tiles::GOES_EAST,
    step: Step::TenMinutes { lag_minutes: 45.0 },
    kept_days: 2.5,
    opacity: 0.92,
    black_is_empty: true,
    note: GEOSTATIONARY_NOTE,
    swatch: [207, 216, 220],
};

pub static GOES_WEST: PictureSpec = PictureSpec {
    name: "Weather satellite: Pacific",
    tiles: tiles::GOES_WEST,
    step: Step::TenMinutes { lag_minutes: 45.0 },
    kept_days: 2.5,
    opacity: 0.92,
    black_is_empty: true,
    note: GEOSTATIONARY_NOTE,
    swatch: [176, 190, 197],
};

pub static HIMAWARI: PictureSpec = PictureSpec {
    name: "Weather satellite: Asia",
    tiles: tiles::HIMAWARI,
    step: Step::TenMinutes { lag_minutes: 45.0 },
    kept_days: 0.15,
    opacity: 0.85,
    black_is_empty: false,
    note: "In infrared, where the coldest and so highest cloud tops are brightest. A \
           new picture every ten minutes; only the last few hours are kept.",
    swatch: [144, 164, 174],
};

pub static PRECIPITATION: PictureSpec = PictureSpec {
    name: "Rain and snow",
    tiles: tiles::PRECIPITATION,
    step: Step::Daily { lag_days: 2.0 },
    kept_days: 14.0,
    opacity: 0.85,
    black_is_empty: false,
    note: "How hard it rained or snowed that day, estimated from many satellites \
           together. Published two days later.",
    swatch: [100, 181, 246],
};

pub static SEA_SURFACE_TEMPERATURE: PictureSpec = PictureSpec {
    name: "Sea surface temperature",
    tiles: tiles::SEA_SURFACE_TEMPERATURE,
    step: Step::Daily { lag_days: 2.0 },
    kept_days: 14.0,
    opacity: 0.85,
    black_is_empty: false,
    note: "Warm water in reds, cold in blues. Published two days later.",
    swatch: [255, 138, 101],
};

/// Every picture Layer, in the order they are listed.
pub static ALL: [&PictureSpec; 6] = [
    &DAILY_TRUE_COLOUR,
    &GOES_EAST,
    &GOES_WEST,
    &HIMAWARI,
    &PRECIPITATION,
    &SEA_SURFACE_TEMPERATURE,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> JulianDate {
        JulianDate::from_utc(2026, 11, day, hour, minute, 0.0)
    }

    #[test]
    fn a_daily_picture_is_the_clocks_day_or_the_latest_whole_one() {
        let daily = Step::Daily { lag_days: 1.0 };
        let now = at(25, 11, 0);
        // Live: today's picture is not complete, so yesterday's is shown.
        assert_eq!(picture_moment(daily, now, now).1, "2026-11-24");
        // The clock moved back three days shows that day's.
        assert_eq!(picture_moment(daily, at(22, 18, 30), now).1, "2026-11-22");
        // And moved forward it cannot show what has not happened.
        assert_eq!(picture_moment(daily, at(27, 0, 0), now).1, "2026-11-24");
        // Published two days late, the latest is the day before yesterday's.
        let late = Step::Daily { lag_days: 2.0 };
        assert_eq!(picture_moment(late, now, now).1, "2026-11-23");
    }

    #[test]
    fn a_ten_minute_picture_is_rounded_down_to_when_one_was_made() {
        let step = Step::TenMinutes { lag_minutes: 45.0 };
        let now = at(25, 11, 3);
        // 45 minutes before 11:03 is 10:18, and the picture before that is 10:10's.
        let (moment, written) = picture_moment(step, now, now);
        assert_eq!(written, "2026-11-25T10:10:00Z");
        assert!((moment.0 - at(25, 10, 10).0).abs() < 1e-9);
        // The clock moved back to a moment on the mark shows that very picture.
        assert_eq!(
            picture_moment(step, at(24, 6, 50), now).1,
            "2026-11-24T06:50:00Z"
        );
        assert_eq!(
            picture_moment(step, at(24, 23, 59), now).1,
            "2026-11-24T23:50:00Z"
        );
    }

    #[test]
    fn the_layer_asks_for_its_picture_at_the_moment_on_show() {
        let mut layer = PictureLayer::new(&GOES_EAST);
        assert_eq!(layer.picture(), None);
        assert!(!layer.has_features());
        layer.set_on(true);
        layer.show(at(24, 6, 55), at(25, 11, 0), Instant::now());
        let Some(Picture::Overlay { tiles, opacity, .. }) = layer.picture() else {
            panic!("no picture");
        };
        assert_eq!(tiles.name, "GOES-East_ABI_GeoColor");
        assert_eq!(tiles.time, "2026-11-24T06:50:00Z");
        assert_eq!(opacity, 0.92);
        let status = layer.status(SystemTime::now()).text;
        assert!(
            status.contains("Showing the picture for 2026-11-24 06:50 UTC."),
            "{status}"
        );

        layer.set_on(false);
        assert_eq!(layer.picture(), None);
    }

    #[test]
    fn the_picture_changes_only_once_the_clock_has_settled_on_a_moment() {
        let mut layer = PictureLayer::new(&GOES_EAST);
        layer.set_on(true);
        let real = at(25, 11, 0);
        let start = Instant::now();
        let showing = |layer: &PictureLayer| layer.showing.clone().unwrap();

        // The first picture is shown at once.
        layer.show(at(24, 6, 0), real, start);
        assert_eq!(showing(&layer), "2026-11-24T06:00:00Z");

        // The clock racing through ten minutes every tenth of a second changes nothing.
        for step in 1_u32..=20 {
            let moment = JulianDate(at(24, 6, 0).0 + f64::from(step) * 10.0 * MINUTE);
            layer.show(
                moment,
                real,
                start + Duration::from_millis(100 * u64::from(step)),
            );
        }
        assert_eq!(showing(&layer), "2026-11-24T06:00:00Z");

        // Once it stops, the picture for where it stopped is shown after a moment.
        let stopped = JulianDate(at(24, 6, 0).0 + 200.0 * MINUTE);
        layer.show(stopped, real, start + Duration::from_millis(2500));
        assert_eq!(showing(&layer), "2026-11-24T06:00:00Z");
        layer.show(stopped, real, start + Duration::from_millis(3300));
        assert_eq!(showing(&layer), "2026-11-24T09:20:00Z");
    }

    #[test]
    fn a_picture_reaches_as_far_back_as_its_source_keeps_it() {
        let layer = PictureLayer::new(&GOES_EAST);
        let now = at(25, 11, 0);
        assert!(layer.time_coverage().covers(at(23, 12, 0), now));
        assert!(!layer.time_coverage().covers(at(20, 12, 0), now));
        assert!(!layer.time_coverage().covers(at(26, 12, 0), now));
    }
}
