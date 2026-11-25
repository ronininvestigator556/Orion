//! Clusters: sorting nearby Features into the squares of a grid, so that each square
//! holding many can be shown as one mark with a count.

/// The size in degrees of the squares at level 0. Each level halves it.
const COARSEST_DEG: f64 = 45.0;
/// The finest level, with squares about a kilometre across. A view closer than
/// this level suits can tell Features apart, and they are not clustered.
pub const FINEST_LEVEL: u8 = 12;

/// A square of the grid: its band of latitude, counted from the South Pole, and its
/// place along that band, counted east from the date line.
pub type Cell = (u32, u32);

/// The size in degrees of the squares at a level.
pub fn cell_size_deg(level: u8) -> f64 {
    COARSEST_DEG / f64::from(1_u32 << level.min(31))
}

/// The level whose squares are at most `spacing` across on screen, where one unit
/// of the screen straight below the camera spans `degrees_per_unit` of the globe.
/// `None` if the view is so close that even the finest squares would be wider.
pub fn level_for(degrees_per_unit: f64, spacing: f64) -> Option<u8> {
    let wanted_deg = degrees_per_unit * spacing;
    if wanted_deg.is_nan() || wanted_deg <= 0.0 {
        return None;
    }
    let level = (COARSEST_DEG / wanted_deg).log2().ceil().max(0.0);
    (level <= f64::from(FINEST_LEVEL)).then_some(level as u8)
}

/// Which square a place falls in at a level.
///
/// Rows are bands of latitude. Each band is cut into as many columns as keep its
/// squares about as wide on the ground as they are tall, so there are fewer towards
/// the poles and a single one at each pole.
pub fn cell(latitude_deg: f64, longitude_deg: f64, level: u8) -> Cell {
    let size = cell_size_deg(level);
    let rows = (180.0 / size).round() as u32;
    let row = (((latitude_deg + 90.0) / size).floor().max(0.0) as u32).min(rows - 1);
    let middle = (f64::from(row) + 0.5) * size - 90.0;
    let columns = ((360.0 * middle.to_radians().cos() / size).round() as u32).max(1);
    let along = (longitude_deg + 180.0).rem_euclid(360.0) / 360.0;
    let column = ((along * f64::from(columns)).floor() as u32).min(columns - 1);
    (row, column)
}

/// Writes a count briefly, as on a Cluster: `87`, `1.2k`, `34k`, `1.5M`.
pub fn format_count(count: u32) -> String {
    match count {
        0..1_000 => count.to_string(),
        1_000..10_000 => format!("{:.1}k", f64::from(count) / 1_000.0),
        10_000..1_000_000 => format!("{:.0}k", f64::from(count) / 1_000.0),
        _ => format!("{:.1}M", f64::from(count) / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squares_halve_with_each_level() {
        assert_eq!(cell_size_deg(0), 45.0);
        assert_eq!(cell_size_deg(4), 2.8125);
        assert!(cell_size_deg(FINEST_LEVEL) < 0.011);
    }

    #[test]
    fn the_level_follows_how_much_of_the_globe_a_pixel_spans() {
        // Where a unit of the screen is 0.065 degrees, marks 44 units apart want
        // squares of about 2.9 degrees: level 4, at 2.8.
        assert_eq!(level_for(0.065, 44.0), Some(4));
        // From very far away the coarsest squares are used.
        assert_eq!(level_for(5.0, 44.0), Some(0));
        // Deeper as the view comes closer, and nothing once Features can be told apart.
        assert_eq!(level_for(0.001, 44.0), Some(10));
        assert_eq!(level_for(0.000_2, 44.0), None);
        assert_eq!(level_for(0.0, 44.0), None);
    }

    #[test]
    fn nearby_places_share_a_square_and_distant_ones_do_not() {
        let paris = cell(48.86, 2.35, 4);
        assert_eq!(cell(48.80, 2.13, 4), paris);
        assert_ne!(cell(51.5, -0.12, 4), paris);
        // Close in, Paris and Versailles part company.
        assert_ne!(cell(48.86, 2.35, 9), cell(48.80, 2.13, 9));
    }

    #[test]
    fn every_place_on_the_globe_has_a_square() {
        for level in [0, 3, FINEST_LEVEL] {
            let rows = (180.0 / cell_size_deg(level)).round() as u32;
            for (latitude, longitude) in [
                (90.0, 0.0),
                (-90.0, 0.0),
                (0.0, 180.0),
                (0.0, -180.0),
                (89.999, 179.999),
                (-45.0, 540.0),
            ] {
                let (row, _) = cell(latitude, longitude, level);
                assert!(row < rows, "{latitude}, {longitude} at level {level}");
            }
        }
        // The date line is one place, whichever side it is written from.
        assert_eq!(cell(10.0, 180.0, 5), cell(10.0, -180.0, 5));
        // Squares near a pole are no narrower on the ground than at the equator:
        // the band nearest the pole has only a few columns.
        let (_, far_east) = cell(89.0, 179.9, 5);
        assert!(far_east < 8, "{far_east}");
    }

    #[test]
    fn counts_are_written_briefly() {
        assert_eq!(format_count(87), "87");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_234), "1.2k");
        assert_eq!(format_count(34_400), "34k");
        assert_eq!(format_count(1_530_000), "1.5M");
    }
}
