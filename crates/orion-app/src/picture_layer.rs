//! Layers that are pictures laid over the Base Map: the day's clouds, the view from
//! a weather satellite, how hard it rained.
//!
//! Such a Layer has no Features. It follows the Simulation Clock: move the clock back
//! and the picture is the one from then, as far back as the Source keeps them.
//!
//! Most pictures are published when their Source gets to it: one made every ten
//! minutes an hour later or two, not always in order and now and then never; a day's
//! rain a day later, or two. So for those the Layer does not reckon which picture is
//! the newest. It asks.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use orion_core::layer::{FeatureCard, Theme, TimeCoverage};
use orion_core::time::JulianDate;
use orion_data::FetchError;
use orion_data::tiles::{self, TileLayer, picture_day, picture_time};

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
/// How long after a failed search for a picture the Source is asked again.
const RETRY: Duration = Duration::from_secs(60);
/// How long after finding only an older picture than the one wanted the Source is
/// asked whether the newer one has been published since.
const RECHECK: Duration = Duration::from_secs(180);
/// The same for a picture made once a day, which is in no such hurry.
const DAILY_RECHECK: Duration = Duration::from_secs(30 * 60);
/// How close to the present the moment wanted must be for a newer picture to be
/// worth asking after again, in days. Further back, what is held will not change.
const RECENT_DAYS: f64 = 0.25;
/// The same for a picture made once a day, which may be published days later.
const DAILY_RECENT_DAYS: f64 = 4.0;

/// How often a picture is made, and how its latest one is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// One a day, served from its first hours and filled in as the day goes on: the
    /// latest whole one is from this many days back.
    Daily { lag_days: f64 },
    /// One a day, published whole when the Source gets to it: the Source is asked
    /// which it holds.
    DailyWhenHeld,
    /// One every ten minutes, published when the Source gets to it: the Source is
    /// asked which it holds.
    TenMinutes,
}

impl Step {
    /// Whether the newest picture is found by asking the Source which it holds.
    fn is_asked(self) -> bool {
        !matches!(self, Self::Daily { .. })
    }

    /// How the Source expects a moment of this picture written.
    pub fn written(self) -> fn(JulianDate) -> String {
        match self {
            Self::Daily { .. } | Self::DailyWhenHeld => picture_day,
            Self::TenMinutes => picture_time,
        }
    }

    /// How long after finding only an older picture the Source is asked again, and
    /// how close to the present, in days, the moment wanted must be for that.
    fn recheck(self) -> (Duration, f64) {
        match self {
            Self::Daily { .. } | Self::DailyWhenHeld => (DAILY_RECHECK, DAILY_RECENT_DAYS),
            Self::TenMinutes => (RECHECK, RECENT_DAYS),
        }
    }
}

/// Finds the newest picture made no later than a moment that its Source really
/// serves; `Ok(None)` if it holds none near then. Called off the main thread.
pub type FindPicture = Arc<
    dyn Fn(&TileLayer, Step, JulianDate) -> Result<Option<JulianDate>, FetchError> + Send + Sync,
>;

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

/// The moment whose picture is wanted when the Simulation Clock is at `at`: the clock's
/// own moment, or the present if the clock is ahead of it, rounded down to when a
/// picture was made. For a picture whose latest whole one is reckoned, that is the
/// picture shown; for the others it is where the search for a picture the Source
/// holds starts. Returned with the way the Source writes it.
pub fn picture_moment(step: Step, at: JulianDate, real_now: JulianDate) -> (JulianDate, String) {
    match step {
        Step::Daily { lag_days } => {
            let moment = JulianDate(at.0.min(real_now.0 - lag_days));
            let day = moment.to_utc();
            let start = JulianDate::from_utc(day.year, day.month, day.day, 0, 0, 0.0);
            (start, picture_day(start))
        }
        Step::DailyWhenHeld => {
            let day = JulianDate(at.0.min(real_now.0)).to_utc();
            let start = JulianDate::from_utc(day.year, day.month, day.day, 0, 0, 0.0);
            (start, picture_day(start))
        }
        Step::TenMinutes => {
            // A second's grace, so a moment on the mark is not rounded down past it.
            let moment = JulianDate(at.0.min(real_now.0) + MINUTE / 60.0);
            let time = moment.to_utc();
            let minute = time.minute / 10 * 10;
            let start =
                JulianDate::from_utc(time.year, time.month, time.day, time.hour, minute, 0.0);
            (start, picture_time(start))
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
    find: FindPicture,
    /// The search for a picture under way, if one is.
    search: Option<Search>,
    /// How the last search ended.
    answer: Option<Answer>,
}

/// A search for the newest picture the Source holds from no later than `moment`.
struct Search {
    wanted: String,
    moment: JulianDate,
    result: Receiver<Result<Option<JulianDate>, FetchError>>,
}

/// How a search ended, and when.
struct Answer {
    /// The moment the search was for, as the Source writes it.
    wanted: String,
    at: Instant,
    outcome: Outcome,
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// The picture for the very moment wanted.
    Found,
    /// An older picture: the one wanted is not held, or not yet.
    FoundOlder,
    /// The Source holds no picture from near the moment wanted.
    Nothing,
    Failed(String),
}

impl PictureLayer {
    /// A Layer that is off. `find` is how it learns which pictures its Source holds.
    pub fn new(spec: &'static PictureSpec, find: FindPicture) -> Self {
        Self {
            spec,
            on: false,
            showing: None,
            wanted: None,
            find,
            search: None,
            answer: None,
        }
    }

    /// Notes the moment the Simulation Clock calls for, and shows its picture once
    /// the clock has called for it long enough. The first moment is acted on at once.
    fn show(&mut self, at: JulianDate, real_now: JulianDate, now: Instant) {
        let (moment, written) = picture_moment(self.spec.step, at, real_now);
        let settled = match &self.wanted {
            Some((wanted, since)) if *wanted == written => now.duration_since(*since) >= SETTLE,
            Some(_) => {
                self.wanted = Some((written.clone(), now));
                false
            }
            None => {
                self.wanted = Some((written.clone(), now));
                true
            }
        };
        if self.spec.step.is_asked() {
            self.take_answer(now);
            if settled && self.search.is_none() && self.should_ask(&written, moment, real_now, now)
            {
                self.ask(written, moment);
            }
        } else if settled {
            self.showing = Some(written);
        }
    }

    /// Takes in the answer to the search under way, if it has arrived.
    fn take_answer(&mut self, now: Instant) {
        let Some(search) = &self.search else { return };
        let outcome = match search.result.try_recv() {
            Ok(Ok(Some(found))) => {
                self.showing = Some(self.spec.step.written()(found));
                if found.0 + MINUTE / 2.0 >= search.moment.0 {
                    Outcome::Found
                } else {
                    Outcome::FoundOlder
                }
            }
            Ok(Ok(None)) => {
                // Whatever was on show is from some other moment, and would mislead.
                self.showing = None;
                Outcome::Nothing
            }
            // A picture already on show stays: it is the best there is to see.
            Ok(Err(error)) => {
                log::warn!("finding a picture for {}: {error}", self.spec.name);
                Outcome::Failed(error.to_string())
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Outcome::Failed("the search stopped".to_owned()),
        };
        self.answer = Some(Answer {
            wanted: search.wanted.clone(),
            at: now,
            outcome,
        });
        self.search = None;
    }

    /// Whether the Source should be asked for the picture of the moment wanted: it has
    /// not been asked, or what it said may have changed since.
    fn should_ask(
        &self,
        wanted: &str,
        moment: JulianDate,
        real_now: JulianDate,
        now: Instant,
    ) -> bool {
        let Some(answer) = &self.answer else {
            return true;
        };
        if answer.wanted != wanted {
            return true;
        }
        let since = now.duration_since(answer.at);
        let (recheck, recent_days) = self.spec.step.recheck();
        let recent = real_now.0 - moment.0 < recent_days;
        match answer.outcome {
            Outcome::Found => false,
            Outcome::FoundOlder | Outcome::Nothing => recent && since >= recheck,
            Outcome::Failed(_) => since >= RETRY,
        }
    }

    /// Starts a search, off the main thread, for the picture of the moment wanted.
    fn ask(&mut self, wanted: String, moment: JulianDate) {
        let (sender, result) = mpsc::channel();
        let find = Arc::clone(&self.find);
        let tiles = self.spec.tiles.clone();
        let step = self.spec.step;
        std::thread::spawn(move || {
            // The Layer may have been switched off meanwhile, which is no matter.
            let _ = sender.send(find(&tiles, step, moment));
        });
        self.search = Some(Search {
            wanted,
            moment,
            result,
        });
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
            self.search = None;
            self.answer = None;
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
        let outcome = self.answer.as_ref().map(|answer| &answer.outcome);
        let mut text = match (&self.showing, outcome) {
            (Some(moment), outcome) => {
                let moment = moment.replace('T', " ").replace(":00Z", " UTC");
                let mut text = format!("From {SOURCE}. Showing the picture for {moment}");
                text.push_str(match outcome {
                    Some(Outcome::FoundOlder) => ", the newest it holds.",
                    Some(Outcome::Failed(_)) => "; asking for a newer one failed.",
                    _ => ".",
                });
                text
            }
            (None, Some(Outcome::Nothing)) => {
                format!("From {SOURCE}, which holds no picture from near this moment.")
            }
            (None, Some(Outcome::Failed(error))) => {
                format!("From {SOURCE}, but finding its picture failed: {error}.")
            }
            (None, _) if self.on && self.spec.step.is_asked() => {
                format!("From {SOURCE}. Asking which pictures it holds.")
            }
            (None, _) => format!("From {SOURCE}."),
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
        // A Layer that is off asks its Source nothing.
        if self.on {
            self.show(at, real_now(), Instant::now());
        }
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

const GEOSTATIONARY_NOTE: &str = "A new picture every ten minutes, published an hour or two \
     after it is taken and now and then not at all. It covers the side of Earth its \
     satellite faces.";

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
    step: Step::TenMinutes,
    kept_days: 2.5,
    opacity: 0.92,
    black_is_empty: true,
    note: GEOSTATIONARY_NOTE,
    swatch: [207, 216, 220],
};

pub static GOES_WEST: PictureSpec = PictureSpec {
    name: "Weather satellite: Pacific",
    tiles: tiles::GOES_WEST,
    step: Step::TenMinutes,
    kept_days: 2.5,
    opacity: 0.92,
    black_is_empty: true,
    note: GEOSTATIONARY_NOTE,
    swatch: [176, 190, 197],
};

pub static HIMAWARI: PictureSpec = PictureSpec {
    name: "Weather satellite: Asia",
    tiles: tiles::HIMAWARI,
    step: Step::TenMinutes,
    kept_days: 2.5,
    opacity: 0.85,
    black_is_empty: false,
    note: "In infrared, where the coldest and so highest cloud tops are brightest. A \
           new picture every ten minutes, published about an hour after it is taken.",
    swatch: [144, 164, 174],
};

pub static PRECIPITATION: PictureSpec = PictureSpec {
    name: "Rain and snow",
    tiles: tiles::PRECIPITATION,
    step: Step::DailyWhenHeld,
    kept_days: 14.0,
    opacity: 0.85,
    black_is_empty: false,
    note: "How hard it rained or snowed that day, estimated from many satellites \
           together. Published a day or two later.",
    swatch: [100, 181, 246],
};

pub static SEA_SURFACE_TEMPERATURE: PictureSpec = PictureSpec {
    name: "Sea surface temperature",
    tiles: tiles::SEA_SURFACE_TEMPERATURE,
    step: Step::DailyWhenHeld,
    kept_days: 14.0,
    opacity: 0.85,
    black_is_empty: false,
    note: "Warm water in reds, cold in blues. Published a day or two later.",
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> JulianDate {
        JulianDate::from_utc(2026, 11, day, hour, minute, 0.0)
    }

    /// A Source that holds the picture for every moment asked about.
    fn holding_everything() -> FindPicture {
        Arc::new(|_, _, moment| Ok(Some(moment)))
    }

    /// A Source whose newest picture is the one from `newest`, counting how often it
    /// is asked.
    fn holding_up_to(newest: JulianDate) -> (FindPicture, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&asked);
        let find: FindPicture = Arc::new(move |_, _, moment: JulianDate| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(Some(JulianDate(moment.0.min(newest.0))))
        });
        (find, asked)
    }

    /// Calls for the same moment until the search under way has been answered.
    fn answered(layer: &mut PictureLayer, clock: JulianDate, real: JulianDate, now: Instant) {
        for _ in 0..400 {
            layer.show(clock, real, now);
            if layer.search.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the search was never answered");
    }

    fn showing(layer: &PictureLayer) -> Option<&str> {
        layer.showing.as_deref()
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
    fn a_ten_minute_picture_is_wanted_for_the_mark_before_the_clock() {
        let step = Step::TenMinutes;
        let now = at(25, 11, 3);
        // Live, the search starts from the present: nothing is reckoned about how
        // late the Source publishes.
        let (moment, written) = picture_moment(step, now, now);
        assert_eq!(written, "2026-11-25T11:00:00Z");
        assert!((moment.0 - at(25, 11, 0).0).abs() < 1e-9);
        // The clock moved back to a moment on the mark wants that very picture.
        assert_eq!(
            picture_moment(step, at(24, 6, 50), now).1,
            "2026-11-24T06:50:00Z"
        );
        assert_eq!(
            picture_moment(step, at(24, 23, 59), now).1,
            "2026-11-24T23:50:00Z"
        );
        // Moved ahead of the present, it wants the present's.
        assert_eq!(
            picture_moment(step, at(27, 0, 0), now).1,
            "2026-11-25T11:00:00Z"
        );
    }

    #[test]
    fn the_layer_asks_for_its_picture_at_the_moment_on_show() {
        let mut layer = PictureLayer::new(&GOES_EAST, holding_everything());
        assert_eq!(layer.picture(), None);
        assert!(!layer.has_features());
        layer.set_on(true);
        answered(&mut layer, at(24, 6, 55), at(25, 11, 0), Instant::now());
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
    fn a_picture_published_late_is_stood_in_for_by_the_newest_one_held() {
        // At 16:40 the Source's newest picture is 15:10's, as it was on the afternoon
        // the Layer was first tried by hand and showed nothing.
        let (find, asked) = holding_up_to(at(25, 15, 10));
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.set_on(true);
        let real = at(25, 16, 40);
        let start = Instant::now();

        // Until the Source has answered there is nothing to show, and it says why.
        layer.show(real, real, start);
        if layer.search.is_some() {
            assert_eq!(layer.picture(), None);
            let status = layer.status(SystemTime::now()).text;
            assert!(
                status.contains("Asking which pictures it holds."),
                "{status}"
            );
        }

        answered(&mut layer, real, real, start);
        assert_eq!(showing(&layer), Some("2026-11-25T15:10:00Z"));
        let status = layer.status(SystemTime::now()).text;
        assert!(
            status.contains("Showing the picture for 2026-11-25 15:10 UTC, the newest it holds."),
            "{status}"
        );
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // It is not asked again every frame, only after a while, in case the picture
        // wanted has been published since.
        answered(&mut layer, real, real, start + Duration::from_secs(100));
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        answered(
            &mut layer,
            real,
            real,
            start + RECHECK + Duration::from_secs(1),
        );
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_picture_found_for_the_very_moment_wanted_is_not_asked_after_again() {
        let (find, asked) = holding_up_to(at(25, 15, 10));
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.set_on(true);
        let real = at(25, 16, 40);
        let start = Instant::now();

        // The clock moved back to yesterday: that picture is held and will stay so.
        answered(&mut layer, at(24, 9, 0), real, start);
        assert_eq!(showing(&layer), Some("2026-11-24T09:00:00Z"));
        answered(
            &mut layer,
            at(24, 9, 0),
            real,
            start + Duration::from_secs(3600),
        );
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_source_holding_nothing_near_the_moment_shows_nothing_and_says_so() {
        let mut layer = PictureLayer::new(&GOES_EAST, Arc::new(|_, _, _| Ok(None)));
        layer.set_on(true);
        let real = at(25, 16, 40);
        answered(&mut layer, real, real, Instant::now());
        assert_eq!(layer.picture(), None);
        let status = layer.status(SystemTime::now()).text;
        assert!(
            status.contains("holds no picture from near this moment"),
            "{status}"
        );
    }

    #[test]
    fn a_failed_search_says_so_keeps_the_picture_on_show_and_is_tried_again() {
        let asked = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&asked);
        // The first search finds its picture; every one after fails.
        let find: FindPicture = Arc::new(move |_, _, moment| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Some(moment))
            } else {
                Err(FetchError::Refused(503))
            }
        });
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.set_on(true);
        let real = at(25, 16, 40);
        let start = Instant::now();
        answered(&mut layer, at(25, 9, 0), real, start);
        assert_eq!(showing(&layer), Some("2026-11-25T09:00:00Z"));

        // The clock moves on to a moment whose search fails.
        layer.show(at(25, 12, 0), real, start + Duration::from_secs(10));
        answered(
            &mut layer,
            at(25, 12, 0),
            real,
            start + Duration::from_secs(12),
        );
        assert_eq!(showing(&layer), Some("2026-11-25T09:00:00Z"));
        let status = layer.status(SystemTime::now()).text;
        assert!(status.contains("asking for a newer one failed"), "{status}");
        assert_eq!(asked.load(Ordering::SeqCst), 2);

        // It is tried again after a rest, not at once.
        answered(
            &mut layer,
            at(25, 12, 0),
            real,
            start + Duration::from_secs(30),
        );
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        let later = start + Duration::from_secs(12) + RETRY + Duration::from_secs(1);
        answered(&mut layer, at(25, 12, 0), real, later);
        assert_eq!(asked.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_failed_first_search_gives_its_reason() {
        let find: FindPicture = Arc::new(|_, _, _| Err(FetchError::Refused(503)));
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.set_on(true);
        let real = at(25, 16, 40);
        answered(&mut layer, real, real, Instant::now());
        assert_eq!(layer.picture(), None);
        let status = layer.status(SystemTime::now()).text;
        assert!(
            status.contains("finding its picture failed: the source answered with HTTP status 503"),
            "{status}"
        );
    }

    #[test]
    fn a_layer_that_is_off_asks_its_source_nothing() {
        let (find, asked) = holding_up_to(at(25, 15, 10));
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.update(at(25, 16, 40), Highlight::default());
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        assert!(layer.search.is_none());
    }

    #[test]
    fn the_picture_changes_only_once_the_clock_has_settled_on_a_moment() {
        let (find, asked) = holding_up_to(at(25, 11, 0));
        let mut layer = PictureLayer::new(&GOES_EAST, find);
        layer.set_on(true);
        let real = at(25, 11, 0);
        let start = Instant::now();

        // The first picture is asked for at once.
        answered(&mut layer, at(24, 6, 0), real, start);
        assert_eq!(showing(&layer), Some("2026-11-24T06:00:00Z"));

        // The clock racing through ten minutes every tenth of a second changes
        // nothing, and asks the Source nothing.
        for step in 1_u32..=20 {
            let moment = JulianDate(at(24, 6, 0).0 + f64::from(step) * 10.0 * MINUTE);
            layer.show(
                moment,
                real,
                start + Duration::from_millis(100 * u64::from(step)),
            );
        }
        assert_eq!(showing(&layer), Some("2026-11-24T06:00:00Z"));
        assert_eq!(asked.load(Ordering::SeqCst), 1);

        // Once it stops, the picture for where it stopped is shown after a moment.
        let stopped = JulianDate(at(24, 6, 0).0 + 200.0 * MINUTE);
        layer.show(stopped, real, start + Duration::from_millis(2500));
        assert_eq!(showing(&layer), Some("2026-11-24T06:00:00Z"));
        assert!(layer.search.is_none());
        layer.show(stopped, real, start + Duration::from_millis(3300));
        answered(
            &mut layer,
            stopped,
            real,
            start + Duration::from_millis(3300),
        );
        assert_eq!(showing(&layer), Some("2026-11-24T09:20:00Z"));
        assert_eq!(asked.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_daily_picture_is_shown_without_asking() {
        let (find, asked) = holding_up_to(at(25, 11, 0));
        let mut layer = PictureLayer::new(&DAILY_TRUE_COLOUR, find);
        layer.set_on(true);
        layer.show(at(25, 11, 0), at(25, 11, 0), Instant::now());
        assert_eq!(showing(&layer), Some("2026-11-24"));
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_daily_picture_that_is_asked_for_is_wanted_for_the_clocks_day() {
        let step = Step::DailyWhenHeld;
        let now = at(25, 18, 10);
        // Live, the search starts from today: nothing is reckoned about how late
        // the Source publishes.
        let (moment, written) = picture_moment(step, now, now);
        assert_eq!(written, "2026-11-25");
        assert!((moment.0 - at(25, 0, 0).0).abs() < 1e-9);
        assert_eq!(picture_moment(step, at(20, 23, 59), now).1, "2026-11-20");
        assert_eq!(picture_moment(step, at(27, 0, 0), now).1, "2026-11-25");
    }

    #[test]
    fn rain_and_sea_temperature_show_the_newest_day_held_not_a_reckoned_one() {
        // On the 25th the Source holds the 24th's picture of each. Reckoning that
        // they are published two days late showed the 23rd's.
        for spec in [&PRECIPITATION, &SEA_SURFACE_TEMPERATURE] {
            let asked = Arc::new(AtomicUsize::new(0));
            let count = Arc::clone(&asked);
            let find: FindPicture = Arc::new(move |_, step, moment: JulianDate| {
                // The Source is told the picture is a daily one, so it asks by day.
                assert_eq!(step, Step::DailyWhenHeld);
                count.fetch_add(1, Ordering::SeqCst);
                Ok(Some(JulianDate(moment.0.min(at(24, 0, 0).0))))
            });
            let mut layer = PictureLayer::new(spec, find);
            layer.set_on(true);
            let real = at(25, 18, 10);
            let start = Instant::now();

            answered(&mut layer, real, real, start);
            assert_eq!(showing(&layer), Some("2026-11-24"));
            let Some(Picture::Overlay { tiles, .. }) = layer.picture() else {
                panic!("no picture");
            };
            assert_eq!(tiles.time, "2026-11-24");
            let status = layer.status(SystemTime::now()).text;
            assert!(
                status.contains("Showing the picture for 2026-11-24, the newest it holds."),
                "{status}"
            );

            // Today's may yet be published, so it is asked after again: every half
            // hour, not every few minutes as for a picture made every ten.
            answered(
                &mut layer,
                real,
                real,
                start + RECHECK + Duration::from_secs(1),
            );
            assert_eq!(asked.load(Ordering::SeqCst), 1);
            let later = start + DAILY_RECHECK + Duration::from_secs(1);
            answered(&mut layer, real, real, later);
            assert_eq!(asked.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn a_daily_picture_of_a_past_day_is_asked_for_once() {
        let (find, asked) = holding_up_to(at(24, 0, 0));
        let mut layer = PictureLayer::new(&PRECIPITATION, find);
        layer.set_on(true);
        let real = at(25, 18, 10);
        let start = Instant::now();
        answered(&mut layer, at(20, 15, 0), real, start);
        assert_eq!(showing(&layer), Some("2026-11-20"));
        let status = layer.status(SystemTime::now()).text;
        assert!(
            status.contains("Showing the picture for 2026-11-20."),
            "{status}"
        );
        answered(
            &mut layer,
            at(20, 15, 0),
            real,
            start + Duration::from_secs(7200),
        );
        assert_eq!(asked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_picture_reaches_as_far_back_as_its_source_keeps_it() {
        let layer = PictureLayer::new(&GOES_EAST, holding_everything());
        let now = at(25, 11, 0);
        assert!(layer.time_coverage().covers(at(23, 12, 0), now));
        assert!(!layer.time_coverage().covers(at(20, 12, 0), now));
        assert!(!layer.time_coverage().covers(at(26, 12, 0), now));
    }
}
