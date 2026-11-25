//! The Simulation Clock: the moment the app is displaying.

use crate::time::JulianDate;

/// How far the Simulation Clock may stray from real time, in days, either way.
///
/// Element Sets are only trustworthy for days around their Epoch, so Positions
/// computed much further out would be fiction.
pub const MAX_OFFSET_DAYS: f64 = 14.0;

/// Rates the clock can run at, in simulated seconds per real second. Negative rates
/// run backwards.
pub const RATES: [f64; 10] = [
    -3_600.0, -600.0, -60.0, -10.0, -1.0, 1.0, 10.0, 60.0, 600.0, 3_600.0,
];

/// The moment being displayed. It follows real time until it is paused, re-timed or
/// moved, and can be sent back to real time at any point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationClock {
    moment: JulianDate,
    /// Simulated seconds per real second.
    rate: f64,
    paused: bool,
    /// True while the clock simply reports real time.
    live: bool,
}

impl SimulationClock {
    /// A clock following real time.
    pub fn new(real_now: JulianDate) -> Self {
        Self {
            moment: real_now,
            rate: 1.0,
            paused: false,
            live: true,
        }
    }

    /// Moves the clock on after `real_elapsed_seconds` of real time and returns the
    /// moment to display. Call once per frame.
    pub fn advance(&mut self, real_now: JulianDate, real_elapsed_seconds: f64) -> JulianDate {
        if self.live {
            self.moment = real_now;
        } else {
            if !self.paused {
                self.moment.0 += self.rate * real_elapsed_seconds / 86_400.0;
            }
            // The allowed window moves with real time, so even a paused clock is
            // eventually carried along at its edge.
            self.moment = clamp(self.moment, real_now);
        }
        self.moment
    }

    pub fn moment(&self) -> JulianDate {
        self.moment
    }

    /// True while the clock is following real time.
    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Simulated seconds per real second.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Days the displayed moment is ahead of real time; negative when behind.
    pub fn offset_days(&self, real_now: JulianDate) -> f64 {
        self.moment.0 - real_now.0
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.live = false;
    }

    /// Steps to the next faster rate, or the next less negative one when running backwards.
    pub fn faster(&mut self) {
        self.step_rate(1);
    }

    /// Steps to the next slower rate, passing through to backwards.
    pub fn slower(&mut self) {
        self.step_rate(-1);
    }

    fn step_rate(&mut self, step: isize) {
        let current = RATES
            .iter()
            .position(|rate| *rate == self.rate)
            .unwrap_or(RATES.len() / 2);
        let next = current.saturating_add_signed(step).min(RATES.len() - 1);
        self.rate = RATES[next];
        self.live = false;
    }

    /// Moves the clock to `moment`, or to the nearest allowed moment if it is too far
    /// from real time.
    pub fn jump_to(&mut self, moment: JulianDate, real_now: JulianDate) {
        self.moment = clamp(moment, real_now);
        self.live = false;
    }

    /// Moves the clock by `days`, positive for later.
    pub fn jump_by(&mut self, days: f64, real_now: JulianDate) {
        self.jump_to(JulianDate(self.moment.0 + days), real_now);
    }

    /// Sends the clock back to following real time at normal speed.
    pub fn return_to_live(&mut self, real_now: JulianDate) {
        *self = Self::new(real_now);
    }
}

fn clamp(moment: JulianDate, real_now: JulianDate) -> JulianDate {
    JulianDate(
        moment
            .0
            .clamp(real_now.0 - MAX_OFFSET_DAYS, real_now.0 + MAX_OFFSET_DAYS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: f64 = 1.0 / 86_400.0;

    fn real_now() -> JulianDate {
        JulianDate::from_utc(2026, 11, 25, 6, 0, 0.0)
    }

    fn later(seconds: f64) -> JulianDate {
        JulianDate(real_now().0 + seconds * SECOND)
    }

    fn assert_close(actual: JulianDate, expected: JulianDate) {
        // Within a millisecond.
        assert!(
            (actual.0 - expected.0).abs() < 1e-3 * SECOND,
            "{actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn a_new_clock_follows_real_time() {
        let mut clock = SimulationClock::new(real_now());
        assert!(clock.is_live());
        assert_eq!(clock.advance(later(5.0), 5.0), later(5.0));
        assert_eq!(clock.offset_days(later(5.0)), 0.0);
    }

    #[test]
    fn a_paused_clock_stands_still() {
        let mut clock = SimulationClock::new(real_now());
        clock.set_paused(true);
        assert!(!clock.is_live());

        assert_eq!(clock.advance(later(30.0), 30.0), real_now());

        // Resuming carries on from where it stopped, not from real time.
        clock.set_paused(false);
        assert_close(clock.advance(later(40.0), 10.0), later(10.0));
    }

    #[test]
    fn the_rate_scales_the_passage_of_time() {
        let mut clock = SimulationClock::new(real_now());
        clock.faster();
        clock.faster();
        assert_eq!(clock.rate(), 60.0);
        // One real second moves the display on by a minute.
        assert_close(clock.advance(later(1.0), 1.0), later(60.0));
    }

    #[test]
    fn slowing_past_normal_speed_runs_backwards() {
        let mut clock = SimulationClock::new(real_now());
        clock.slower();
        assert_eq!(clock.rate(), -1.0);
        assert_close(clock.advance(later(2.0), 2.0), later(-2.0));

        // The ladder stops at its ends.
        for _ in 0..20 {
            clock.slower();
        }
        assert_eq!(clock.rate(), -3_600.0);
        for _ in 0..20 {
            clock.faster();
        }
        assert_eq!(clock.rate(), 3_600.0);
    }

    #[test]
    fn the_clock_cannot_stray_more_than_two_weeks_from_real_time() {
        let mut clock = SimulationClock::new(real_now());

        clock.jump_to(JulianDate(real_now().0 + 100.0), real_now());
        assert_eq!(clock.offset_days(real_now()), MAX_OFFSET_DAYS);

        clock.jump_by(-1_000.0, real_now());
        assert_eq!(clock.offset_days(real_now()), -MAX_OFFSET_DAYS);

        // Running backwards at the early edge goes nowhere.
        clock.slower();
        clock.advance(later(10.0), 10.0);
        assert_close(clock.moment(), JulianDate(later(10.0).0 - MAX_OFFSET_DAYS));
    }

    #[test]
    fn jumps_move_the_clock_and_leave_it_running() {
        let mut clock = SimulationClock::new(real_now());
        clock.jump_by(1.0, real_now());
        assert!(!clock.is_live());
        assert_eq!(clock.offset_days(real_now()), 1.0);
        // It keeps running at normal speed from the new moment.
        assert_close(
            clock.advance(later(3.0), 3.0),
            JulianDate(later(3.0).0 + 1.0),
        );
    }

    #[test]
    fn returning_to_live_undoes_every_change() {
        let mut clock = SimulationClock::new(real_now());
        clock.faster();
        clock.set_paused(true);
        clock.jump_by(-3.0, real_now());

        clock.return_to_live(later(7.0));
        assert_eq!(clock, SimulationClock::new(later(7.0)));
    }
}
