//! Moments in time and Earth's rotation.

/// A moment in UTC as a Julian Date: a continuous count of days, with the fraction
/// giving the time of day. Day boundaries fall at noon, so `.0` is 12:00 UTC.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct JulianDate(pub f64);

impl JulianDate {
    /// 1 January 2000, 12:00, the reference moment for most astronomical formulas.
    pub const J2000: Self = Self(2_451_545.0);

    /// 1 January 1970, 00:00 UTC.
    pub const UNIX_EPOCH: Self = Self(2_440_587.5);

    pub fn from_unix_seconds(seconds: f64) -> Self {
        Self(Self::UNIX_EPOCH.0 + seconds / 86_400.0)
    }

    /// Builds a moment from a Gregorian calendar date and a time of day in UTC.
    pub fn from_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: f64) -> Self {
        // Count whole days with March as the first month, so the leap day falls at the
        // end of the counting year and month lengths follow a regular pattern.
        let shift = (14 - month as i64) / 12;
        let y = year as i64 + 4800 - shift;
        let m = month as i64 + 12 * shift - 3;
        let day_number =
            day as i64 + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32_045;

        let day_fraction =
            (hour as f64 - 12.0) / 24.0 + minute as f64 / 1_440.0 + second / 86_400.0;
        Self(day_number as f64 + day_fraction)
    }

    pub fn days_since_j2000(self) -> f64 {
        self.0 - Self::J2000.0
    }

    /// Angle in radians, in `0..TAU`, by which the Earth-fixed frame is rotated eastward
    /// from the inertial frame at this moment (Greenwich Mean Sidereal Time).
    pub fn greenwich_sidereal_angle(self) -> f64 {
        let days = self.days_since_j2000();
        let centuries = days / 36_525.0;
        let degrees =
            280.460_618_37 + 360.985_647_366_29 * days + 0.000_387_933 * centuries * centuries;
        degrees.rem_euclid(360.0).to_radians()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_dates_convert_to_known_julian_dates() {
        assert_eq!(
            JulianDate::from_utc(2000, 1, 1, 12, 0, 0.0),
            JulianDate::J2000
        );
        assert_eq!(
            JulianDate::from_utc(1970, 1, 1, 0, 0, 0.0),
            JulianDate::UNIX_EPOCH
        );
        // Leap day, and a date after it in the same year.
        assert_eq!(JulianDate::from_utc(2024, 2, 29, 0, 0, 0.0).0, 2_460_369.5);
        assert_eq!(JulianDate::from_utc(2024, 3, 1, 0, 0, 0.0).0, 2_460_370.5);
    }

    #[test]
    fn unix_seconds_match_calendar_dates() {
        // 2024-03-20 03:06:00 UTC.
        let from_unix = JulianDate::from_unix_seconds(1_710_903_960.0);
        let from_calendar = JulianDate::from_utc(2024, 3, 20, 3, 6, 0.0);
        assert!((from_unix.0 - from_calendar.0).abs() < 1e-9);
    }

    #[test]
    fn sidereal_angle_matches_published_example() {
        // Vallado, "Fundamentals of Astrodynamics and Applications", example 3-5:
        // 20 August 1992, 12:14 UT1 gives 152.578787810 degrees.
        let angle = JulianDate::from_utc(1992, 8, 20, 12, 14, 0.0).greenwich_sidereal_angle();
        assert!((angle.to_degrees() - 152.578_787_810).abs() < 1e-4);
    }
}
