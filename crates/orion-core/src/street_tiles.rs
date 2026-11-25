//! The grid of tiles a street map is published in.
//!
//! It is the grid nearly every web map uses. The world is drawn as a square in the
//! Mercator projection, which stretches as it nears the poles and stops at about 85
//! degrees north and south. Level 0 is that square as one tile; each deeper level
//! splits every tile into four. A tile is 256 pixels across. Columns count east from
//! longitude 180 west and rows south from the top edge.
//!
//! A tile's east and west edges are evenly spaced in longitude. Its rows are not
//! evenly spaced in latitude, so a tile is laid on the globe by working out where
//! each point of it falls, not by stretching it between its corners.

use std::f64::consts::PI;

use crate::tiles::TileKey;

/// Width and height of a tile, in pixels.
pub const TILE_PIXELS: u32 = 256;
/// The latitude the grid stops at, north and south, in degrees.
pub const MAX_LATITUDE_DEG: f64 = 85.051_128_779_806_6;

/// The part of the world a tile covers, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreetTileBounds {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

/// How many tiles a level has across, and down.
pub fn tiles_across(level: u8) -> u32 {
    1 << level
}

/// The latitude, in degrees, of the line a fraction `down` of the way from the top
/// edge of the grid (0) to the bottom (1).
pub fn latitude_at(down: f64) -> f64 {
    (PI * (1.0 - 2.0 * down)).sinh().atan().to_degrees()
}

/// How far down the grid a latitude falls, from 0 at the top edge to 1 at the bottom.
/// Latitudes beyond the grid count as its edge.
pub fn down_at(latitude_deg: f64) -> f64 {
    let latitude = latitude_deg.clamp(-MAX_LATITUDE_DEG, MAX_LATITUDE_DEG);
    (0.5 - latitude.to_radians().tan().asinh() / (2.0 * PI)).clamp(0.0, 1.0)
}

/// The part of the world a tile covers.
pub fn bounds(key: TileKey) -> StreetTileBounds {
    let across = f64::from(tiles_across(key.level));
    StreetTileBounds {
        west: -180.0 + 360.0 * f64::from(key.col) / across,
        east: -180.0 + 360.0 * f64::from(key.col + 1) / across,
        south: latitude_at(f64::from(key.row + 1) / across),
        north: latitude_at(f64::from(key.row) / across),
    }
}

/// The tile at `level` that contains the given place.
pub fn containing(level: u8, latitude_deg: f64, longitude_deg: f64) -> TileKey {
    let across = tiles_across(level);
    let east = (longitude_deg + 180.0).rem_euclid(360.0) / 360.0;
    let cell = |fraction: f64| ((fraction * f64::from(across)).floor() as u32).min(across - 1);
    TileKey {
        level,
        row: cell(down_at(latitude_deg)),
        col: cell(east),
    }
}

/// The deepest level whose pixels are no finer than `radians_per_pixel` on the ground
/// at the given latitude, so a tile is never drawn smaller than it was made. Tiles
/// nearer the poles cover less ground, so the same view needs a shallower level there.
pub fn level_for_resolution(radians_per_pixel: f64, latitude_deg: f64, max_level: u8) -> u8 {
    let latitude = latitude_deg.clamp(-MAX_LATITUDE_DEG, MAX_LATITUDE_DEG);
    let level_zero = 2.0 * PI * latitude.to_radians().cos() / f64::from(TILE_PIXELS);
    let level = (level_zero / radians_per_pixel.max(1e-12)).log2().floor();
    level.clamp(0.0, f64::from(max_level)) as u8
}

/// Every tile at `level` that touches the circle on the globe centred on the given
/// place with the given radius, both in degrees. The result covers the circle's
/// bounding box, so it may include a few tiles just outside the circle.
pub fn tiles_covering(
    latitude_deg: f64,
    longitude_deg: f64,
    radius_deg: f64,
    level: u8,
) -> Vec<TileKey> {
    let across = tiles_across(level);
    let north = latitude_deg + radius_deg;
    let south = latitude_deg - radius_deg;
    let first_row = containing(level, north.min(90.0), 0.0).row;
    let last_row = containing(level, south.max(-90.0), 0.0).row;

    // How far east and west the circle reaches. Around a pole it reaches every longitude.
    let reaches_pole = north >= 90.0 || south <= -90.0 || radius_deg >= 90.0;
    let half_width = if reaches_pole {
        180.0
    } else {
        let ratio = radius_deg.to_radians().sin() / latitude_deg.to_radians().cos();
        ratio.clamp(-1.0, 1.0).asin().to_degrees()
    };
    let columns: Vec<u32> = if half_width >= 180.0 {
        (0..across).collect()
    } else {
        let west = containing(level, 0.0, longitude_deg - half_width).col;
        let east = containing(level, 0.0, longitude_deg + half_width).col;
        if west <= east {
            (west..=east).collect()
        } else {
            // The circle straddles longitude 180, where the columns start again.
            (west..across).chain(0..=east).collect()
        }
    };

    (first_row..=last_row)
        .flat_map(|row| {
            columns.iter().map(move |col| TileKey {
                level,
                row,
                col: *col,
            })
        })
        .collect()
}

/// The tile one level up that this tile lies inside, or `None` at level 0.
pub fn parent(key: TileKey) -> Option<TileKey> {
    key.parent()
}

/// Where the points of a tile fall on the globe: a square of `cells` by `cells`
/// squares, so `cells + 1` points each way, row by row from the north-west corner.
/// Each point is a unit vector from Earth's centre in the Earth-fixed frame, with how
/// far across and down the tile's picture it lies, from 0 to 1.
pub fn patch(key: TileKey, cells: u16) -> Vec<([f64; 3], [f32; 2])> {
    let across = f64::from(tiles_across(key.level));
    let side = usize::from(cells) + 1;
    let mut points = Vec::with_capacity(side * side);
    for row in 0..=cells {
        let v = f64::from(row) / f64::from(cells);
        let latitude = latitude_at((f64::from(key.row) + v) / across).to_radians();
        for col in 0..=cells {
            let u = f64::from(col) / f64::from(cells);
            let longitude = (-180.0 + 360.0 * (f64::from(key.col) + u) / across).to_radians();
            points.push((
                [
                    latitude.cos() * longitude.cos(),
                    latitude.cos() * longitude.sin(),
                    latitude.sin(),
                ],
                [u as f32, v as f32],
            ));
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(level: u8, col: u32, row: u32) -> TileKey {
        TileKey { level, row, col }
    }

    #[test]
    fn the_grid_stops_short_of_the_poles_and_is_not_evenly_spaced() {
        assert!((latitude_at(0.0) - MAX_LATITUDE_DEG).abs() < 1e-9);
        assert!(latitude_at(0.5).abs() < 1e-12);
        assert!((latitude_at(1.0) + MAX_LATITUDE_DEG).abs() < 1e-9);
        // A quarter of the way down is far north of 42.5 degrees.
        assert!((latitude_at(0.25) - 66.513_260_443).abs() < 1e-6);
        for latitude in [-80.0, -33.87, 0.0, 48.86, 84.9] {
            assert!((latitude_at(down_at(latitude)) - latitude).abs() < 1e-9);
        }
        // Beyond the grid counts as its edge.
        assert_eq!(down_at(89.0), 0.0);
        assert_eq!(down_at(-90.0), 1.0);
    }

    #[test]
    fn well_known_places_fall_in_the_tiles_every_web_map_gives_them() {
        assert_eq!(containing(14, 48.8566, 2.3522), key(14, 8299, 5636));
        assert_eq!(containing(10, -33.8688, 151.2093), key(10, 942, 614));
        assert_eq!(containing(16, 40.7128, -74.0060), key(16, 19295, 24640));
        assert_eq!(containing(0, 12.0, 34.0), key(0, 0, 0));
        // Longitude wraps, and the last column and row are not overrun.
        assert_eq!(containing(3, 0.0, 180.0).col, 0);
        assert_eq!(containing(3, 0.0, 179.999).col, 7);
        assert_eq!(containing(3, -90.0, 0.0).row, 7);
    }

    #[test]
    fn a_tile_covers_the_ground_between_its_neighbours() {
        let paris = bounds(key(14, 8299, 5636));
        assert!((paris.west - 2.351_074_218_75).abs() < 1e-12);
        assert!((paris.east - 2.373_046_875).abs() < 1e-12);
        assert!((paris.south - 48.850_258_199_721).abs() < 1e-9);
        assert!((paris.north - 48.864_714_761_803).abs() < 1e-9);
        // It shares its edges with the tiles beside and below it.
        assert_eq!(bounds(key(14, 8300, 5636)).west, paris.east);
        assert_eq!(bounds(key(14, 8299, 5637)).north, paris.south);
        // And lies inside its parent.
        let parent = bounds(parent(key(14, 8299, 5636)).unwrap());
        assert!(parent.west <= paris.west && parent.east >= paris.east);
        assert!(parent.south <= paris.south && parent.north >= paris.north);
        assert_eq!(super::parent(key(0, 0, 0)), None);
    }

    #[test]
    fn the_level_deepens_as_the_view_sharpens_and_is_shallower_nearer_the_poles() {
        let pixel = |level: i32| 2.0 * PI / (256.0 * 2f64.powi(level));
        assert_eq!(level_for_resolution(pixel(10), 0.0, 19), 10);
        assert_eq!(level_for_resolution(pixel(10) * 0.99, 0.0, 19), 10);
        assert_eq!(level_for_resolution(pixel(10) * 1.01, 0.0, 19), 9);
        assert_eq!(level_for_resolution(pixel(16), 0.0, 19), 16);
        // At 60 degrees a tile covers half the ground it does at the equator.
        assert_eq!(level_for_resolution(pixel(10), 60.0, 19), 9);
        // Never past what is published, nor below the whole world.
        assert_eq!(level_for_resolution(pixel(25), 0.0, 19), 19);
        assert_eq!(level_for_resolution(1.0, 0.0, 19), 0);
    }

    #[test]
    fn the_tiles_round_a_place_include_its_own_and_its_neighbours() {
        let tiles = tiles_covering(48.8566, 2.3522, 0.02, 14);
        assert!(tiles.contains(&key(14, 8299, 5636)));
        assert!(tiles.len() >= 4 && tiles.len() <= 12, "{}", tiles.len());
        assert!(tiles.iter().all(|tile| tile.level == 14));
        // No tile is listed twice.
        let mut unique = tiles.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), tiles.len());
    }

    #[test]
    fn a_view_across_the_date_line_or_a_pole_wraps_round() {
        let fiji = tiles_covering(-17.0, 179.5, 2.0, 5);
        let columns: Vec<u32> = fiji.iter().map(|tile| tile.col).collect();
        assert!(columns.contains(&31) && columns.contains(&0), "{columns:?}");
        assert!(!columns.contains(&15));
        // Looking down on the North Pole takes in every longitude.
        let arctic = tiles_covering(89.0, 0.0, 10.0, 3);
        let columns: std::collections::BTreeSet<u32> = arctic.iter().map(|t| t.col).collect();
        assert_eq!(columns.len(), 8);
        // And only the northernmost rows: 79 degrees north is just inside the second.
        assert!(arctic.iter().all(|tile| tile.row <= 1));
        // The whole world at level 1 is its four tiles.
        assert_eq!(tiles_covering(0.0, 0.0, 90.0, 1).len(), 4);
    }

    #[test]
    fn a_tiles_points_lie_on_the_globe_from_its_north_west_corner() {
        let tile = key(14, 8299, 5636);
        let points = patch(tile, 4);
        assert_eq!(points.len(), 25);
        for (position, _) in &points {
            let length = position.iter().map(|part| part * part).sum::<f64>().sqrt();
            assert!((length - 1.0).abs() < 1e-12);
        }
        let place = |index: usize| {
            let [x, y, z] = points[index].0;
            (z.asin().to_degrees(), y.atan2(x).to_degrees())
        };
        let edges = bounds(tile);
        let (north, west) = place(0);
        assert!((north - edges.north).abs() < 1e-9 && (west - edges.west).abs() < 1e-9);
        let (south, east) = place(24);
        assert!((south - edges.south).abs() < 1e-9 && (east - edges.east).abs() < 1e-9);
        assert_eq!(points[0].1, [0.0, 0.0]);
        assert_eq!(points[4].1, [1.0, 0.0]);
        assert_eq!(points[24].1, [1.0, 1.0]);
        // Across a whole level 1 tile the middle row is not midway in latitude.
        let world = patch(key(1, 0, 0), 2);
        let middle = world[3].0[2].asin().to_degrees();
        assert!((middle - 66.513_260_443).abs() < 1e-6, "{middle}");
    }
}
