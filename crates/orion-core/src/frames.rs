//! Conversion between the inertial and Earth-fixed frames described in the crate documentation.

use crate::time::JulianDate;

/// Expresses an inertial-frame vector in the Earth-fixed frame at the given moment.
///
/// Only Earth's daily rotation is applied. The wobble of the rotation axis (polar
/// motion) is ignored; it moves points by at most about 15 metres at the surface.
pub fn inertial_to_earth_fixed([x, y, z]: [f64; 3], at: JulianDate) -> [f64; 3] {
    // Earth has turned eastward by the sidereal angle, so in the Earth-fixed frame
    // everything inertial appears turned westward by the same amount.
    let (sin, cos) = at.greenwich_sidereal_angle().sin_cos();
    [x * cos + y * sin, -x * sin + y * cos, z]
}

/// A position relative to Earth's surface, on the WGS 84 ellipsoid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geodetic {
    /// Degrees north of the equator.
    pub latitude_deg: f64,
    /// Degrees east of the Greenwich meridian, from -180 to 180.
    pub longitude_deg: f64,
    /// Height above the ellipsoid, which is close to height above mean sea level.
    pub altitude_km: f64,
}

/// Converts a position in kilometres in the Earth-fixed frame to latitude, longitude and altitude.
///
/// Earth is slightly flattened, so "directly below" is not along the line to Earth's
/// centre; the latitude is refined until the surface point and its height agree.
pub fn geodetic([x, y, z]: [f64; 3]) -> Geodetic {
    const FLATTENING: f64 = 1.0 / 298.257_223_563;
    const ECCENTRICITY_SQUARED: f64 = FLATTENING * (2.0 - FLATTENING);
    let radius = crate::EARTH_EQUATORIAL_RADIUS_KM;

    let distance_from_axis = x.hypot(y);
    let mut latitude = z.atan2(distance_from_axis);
    let mut curvature_radius = radius;
    // Converges to well under a millimetre in a handful of rounds at any altitude.
    for _ in 0..8 {
        let sin = latitude.sin();
        curvature_radius = radius / (1.0 - ECCENTRICITY_SQUARED * sin * sin).sqrt();
        latitude = (z + curvature_radius * ECCENTRICITY_SQUARED * sin).atan2(distance_from_axis);
    }

    // Near the poles the usual height formula divides by a vanishing cosine.
    let altitude_km = if latitude.abs() < 1.0 {
        distance_from_axis / latitude.cos() - curvature_radius
    } else {
        z / latitude.sin() - curvature_radius * (1.0 - ECCENTRICITY_SQUARED)
    };

    Geodetic {
        latitude_deg: latitude.to_degrees(),
        longitude_deg: y.atan2(x).to_degrees(),
        altitude_km,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geodetic_position_matches_published_example() {
        // Vallado, example 3-3.
        let position = geodetic([6_524.834, 6_862.875, 6_448.296]);
        assert!(
            (position.latitude_deg - 34.352_496).abs() < 1e-4,
            "{position:?}"
        );
        assert!(
            (position.longitude_deg - 46.446_4).abs() < 1e-3,
            "{position:?}"
        );
        assert!(
            (position.altitude_km - 5_085.22).abs() < 0.05,
            "{position:?}"
        );
    }

    #[test]
    fn geodetic_position_at_the_surface_and_the_poles() {
        let equator = geodetic([crate::EARTH_EQUATORIAL_RADIUS_KM, 0.0, 0.0]);
        assert!(equator.latitude_deg.abs() < 1e-9 && equator.altitude_km.abs() < 1e-6);

        // The polar radius is about 21 km shorter than the equatorial one.
        let pole = geodetic([0.0, 0.0, 6_356.752_314 + 400.0]);
        assert!((pole.latitude_deg - 90.0).abs() < 1e-6, "{pole:?}");
        assert!((pole.altitude_km - 400.0).abs() < 1e-3, "{pole:?}");
    }

    #[test]
    fn rotation_matches_published_example() {
        // Vallado, example 3-15: 6 April 2004, 07:51:28.386009 UTC, with UT1 0.4399619
        // seconds behind UTC. The rotation uses UT1.
        let at = JulianDate::from_utc(2004, 4, 6, 7, 51, 28.386_009 - 0.439_961_9);
        let inertial = [5_094.180_162_10, 6_127.644_659_50, 6_380.344_532_70];
        let expected = [-1_033.475_031_30, 7_901.305_585_60, 6_380.344_532_70];

        let actual = inertial_to_earth_fixed(inertial, at);
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() < 0.01,
                "axis {axis}: {} vs {}",
                actual[axis],
                expected[axis]
            );
        }
    }
}
