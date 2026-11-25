//! Moments in time and Earth's rotation.

/// A moment in UTC as a Julian Date: a continuous count of days, with the fraction
/// giving the time of day. Day boundaries fall at noon, so `.0` is 12:00 UTC.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct JulianDate(pub f64);

/// A Gregorian calendar date and time of day in UTC.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalendarTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
}

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

    /// Reads a UTC date and time written as `YYYY-MM-DD HH:MM` or `YYYY-MM-DD HH:MM:SS`.
    /// A `T` may stand in for the space. Returns `None` for anything else, including
    /// dates that do not exist.
    pub fn parse_utc(text: &str) -> Option<Self> {
        let (date, time) = text.trim().split_once([' ', 'T'])?;

        let mut date = date.split('-');
        let year: i32 = date.next()?.parse().ok()?;
        let month: u32 = date.next()?.parse().ok()?;
        let day: u32 = date.next()?.parse().ok()?;

        let mut time = time.trim().split(':');
        let hour: u32 = time.next()?.parse().ok()?;
        let minute: u32 = time.next()?.parse().ok()?;
        let second: u32 = match time.next() {
            Some(second) => second.parse().ok()?,
            None => 0,
        };
        if date.next().is_some() || time.next().is_some() {
            return None;
        }

        let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap_year => 29,
            2 => 28,
            _ => return None,
        };
        let valid = (1..=days_in_month).contains(&day) && hour < 24 && minute < 60 && second < 60;
        valid.then(|| Self::from_utc(year, month, day, hour, minute, second as f64))
    }

    /// The Gregorian calendar date and time of day of this moment, in UTC.
    pub fn to_utc(self) -> CalendarTime {
        // Shift by half a day so whole numbers fall at midnight, then split.
        let shifted = self.0 + 0.5;
        let day_number = shifted.floor();
        let seconds_of_day = (shifted - day_number) * 86_400.0;

        // Richards' algorithm for turning a day count into a Gregorian date.
        let j = day_number as i64;
        let f = j + 1401 + (((4 * j + 274_277) / 146_097) * 3) / 4 - 38;
        let e = 4 * f + 3;
        let g = (e % 1461) / 4;
        let h = 5 * g + 2;
        let day = (h % 153) / 5 + 1;
        let month = (h / 153 + 2) % 12 + 1;
        let year = e / 1461 - 4716 + (14 - month) / 12;

        let hour = (seconds_of_day / 3_600.0).floor();
        let minute = ((seconds_of_day - hour * 3_600.0) / 60.0).floor();
        CalendarTime {
            year: year as i32,
            month: month as u32,
            day: day as u32,
            hour: hour as u32,
            minute: minute as u32,
            second: seconds_of_day - hour * 3_600.0 - minute * 60.0,
        }
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
    fn julian_dates_convert_back_to_calendar_dates() {
        for (year, month, day, hour, minute) in [
            (2000, 1, 1, 12, 0),
            (1970, 1, 1, 0, 0),
            (2024, 2, 29, 23, 59),
            (2024, 3, 1, 0, 0),
            (2026, 11, 24, 19, 13),
            (2026, 12, 31, 6, 30),
        ] {
            let calendar = JulianDate::from_utc(year, month, day, hour, minute, 30.0).to_utc();
            assert_eq!(
                (
                    calendar.year,
                    calendar.month,
                    calendar.day,
                    calendar.hour,
                    calendar.minute
                ),
                (year, month, day, hour, minute)
            );
            assert!((calendar.second - 30.0).abs() < 1e-3, "{calendar:?}");
        }
    }

    #[test]
    fn written_times_are_read_as_utc() {
        let expected = JulianDate::from_utc(2026, 11, 25, 18, 30, 0.0);
        assert_eq!(JulianDate::parse_utc("2026-11-25 18:30"), Some(expected));
        assert_eq!(
            JulianDate::parse_utc(" 2026-11-25T18:30:00 "),
            Some(expected)
        );
        assert_eq!(
            JulianDate::parse_utc("2024-02-29 00:00:59"),
            Some(JulianDate::from_utc(2024, 2, 29, 0, 0, 59.0))
        );
    }

    #[test]
    fn nonsense_times_are_not_read() {
        for text in [
            "",
            "tomorrow",
            "2026-11-25",
            "2026-11-25 18",
            "2026-13-01 00:00",
            "2026-02-29 00:00",
            "2026-11-31 00:00",
            "2026-11-25 24:00",
            "2026-11-25 18:60",
            "2026-11-25 18:30:00:00",
            "2026-11-25-01 18:30",
        ] {
            assert_eq!(JulianDate::parse_utc(text), None, "{text:?}");
        }
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
