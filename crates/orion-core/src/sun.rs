//! Where the Sun is, as seen from Earth's centre.
//!
//! Uses the low-precision solar position from the Astronomical Almanac, which is good
//! to about 0.01 degrees between 1950 and 2050. That is far finer than a day/night
//! terminator needs.

use crate::frames::inertial_to_earth_fixed;
use crate::time::JulianDate;

/// A place on Earth's surface, in degrees. North and east are positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfacePoint {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}

/// Unit vector from Earth's centre to the Sun in the Earth-fixed frame
/// (see the crate documentation for the axes).
pub fn sun_direction(at: JulianDate) -> [f64; 3] {
    inertial_to_earth_fixed(sun_direction_inertial(at), at)
}

/// The point on Earth's surface where the Sun is directly overhead.
pub fn subsolar_point(at: JulianDate) -> SurfacePoint {
    let [x, y, z] = sun_direction(at);
    SurfacePoint {
        latitude_deg: z.asin().to_degrees(),
        longitude_deg: y.atan2(x).to_degrees(),
    }
}

/// Unit vector to the Sun in the inertial frame: +X towards the March equinox,
/// +Z along Earth's rotation axis.
fn sun_direction_inertial(at: JulianDate) -> [f64; 3] {
    let days = at.days_since_j2000();

    let mean_longitude = (280.460 + 0.985_647_4 * days).to_radians();
    let mean_anomaly = (357.528 + 0.985_600_3 * days).to_radians();
    // Earth's orbit is slightly elliptical, so the Sun runs ahead of or behind its mean position.
    let ecliptic_longitude = mean_longitude
        + 1.915_f64.to_radians() * mean_anomaly.sin()
        + 0.020_f64.to_radians() * (2.0 * mean_anomaly).sin();
    // Tilt of Earth's axis relative to its orbit.
    let obliquity = (23.439 - 0.000_000_4 * days).to_radians();

    [
        ecliptic_longitude.cos(),
        obliquity.cos() * ecliptic_longitude.sin(),
        obliquity.sin() * ecliptic_longitude.sin(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inertial_direction_matches_published_example() {
        // Vallado, example 5-1: 2 April 2006, 00:00 UTC. The Sun's position is
        // (0.9771945, 0.1924424, 0.0834308) AU, normalised here to a unit vector.
        let expected = [0.977_730, 0.192_548, 0.083_477];
        let actual = sun_direction_inertial(JulianDate::from_utc(2006, 4, 2, 0, 0, 0.0));
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() < 2e-4,
                "axis {axis}: {} vs {}",
                actual[axis],
                expected[axis]
            );
        }
    }

    #[test]
    fn direction_is_a_unit_vector() {
        let [x, y, z] = sun_direction(JulianDate::from_utc(2026, 11, 25, 5, 30, 0.0));
        assert!(((x * x + y * y + z * z).sqrt() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn sun_is_over_the_equator_at_the_march_equinox() {
        // The 2024 March equinox fell at 03:06 UTC on 20 March.
        let point = subsolar_point(JulianDate::from_utc(2024, 3, 20, 3, 6, 0.0));
        assert!(point.latitude_deg.abs() < 0.05, "{point:?}");
    }

    #[test]
    fn sun_reaches_the_tropic_at_the_june_solstice() {
        // The 2024 June solstice fell at 20:51 UTC on 20 June.
        let point = subsolar_point(JulianDate::from_utc(2024, 6, 20, 20, 51, 0.0));
        assert!((point.latitude_deg - 23.44).abs() < 0.05, "{point:?}");
    }

    #[test]
    fn subsolar_longitude_at_noon_follows_the_equation_of_time() {
        // At 12:00 UTC the Sun is near the Greenwich meridian, offset by the equation
        // of time at 0.25 degrees per minute.
        //
        // 20 March: sundials run about 7.4 minutes slow, so the Sun has not yet
        // reached Greenwich and is still to the east.
        let march = subsolar_point(JulianDate::from_utc(2024, 3, 20, 12, 0, 0.0));
        assert!((march.longitude_deg - 1.86).abs() < 0.15, "{march:?}");

        // 3 November: sundials run about 16.4 minutes fast, so the Sun has already
        // passed Greenwich and is to the west.
        let november = subsolar_point(JulianDate::from_utc(2024, 11, 3, 12, 0, 0.0));
        assert!((november.longitude_deg + 4.11).abs() < 0.15, "{november:?}");
    }
}
