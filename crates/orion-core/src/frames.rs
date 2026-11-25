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

#[cfg(test)]
mod tests {
    use super::*;

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
