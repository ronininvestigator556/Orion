//! The grid of tiles the Base Map is streamed in.
//!
//! The grid is the one NASA's GIBS service uses for plain latitude/longitude imagery.
//! Level 0 has tiles 288 degrees across, and each deeper level halves that. A tile is
//! 512 pixels across, so level 0 shows 0.5625 degrees per pixel. Rows count south from
//! the North Pole and columns east from longitude 180 west. Tiles on the east and
//! south edges hang over the edge of the world; the overhang holds no picture.

/// Width and height of a level 0 tile, in degrees.
pub const LEVEL_ZERO_SPAN_DEG: f64 = 288.0;
/// Width and height of a tile, in pixels.
pub const TILE_PIXELS: u32 = 512;

/// One tile of the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub level: u8,
    pub row: u32,
    pub col: u32,
}

/// The part of the world a tile covers, in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileBounds {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
    /// The fraction of the tile's width and height that lies inside the world. It is
    /// below 1 only for tiles hanging over the east or south edge.
    pub filled: [f64; 2],
}

/// Width and height in degrees of a tile at the given level.
pub fn span_deg(level: u8) -> f64 {
    LEVEL_ZERO_SPAN_DEG / f64::from(1u32 << level)
}

/// How many columns and rows of tiles the given level has.
pub fn grid_size(level: u8) -> (u32, u32) {
    let span = span_deg(level);
    ((360.0 / span).ceil() as u32, (180.0 / span).ceil() as u32)
}

/// The deepest level whose pixels are no finer than `degrees_per_pixel`, so a tile is
/// never drawn smaller than it was made. Clamped to `min_level..=max_level`.
pub fn level_for_resolution(degrees_per_pixel: f64, min_level: u8, max_level: u8) -> u8 {
    let level_zero = LEVEL_ZERO_SPAN_DEG / f64::from(TILE_PIXELS);
    let level = (level_zero / degrees_per_pixel.max(1e-12)).log2().floor();
    level.clamp(f64::from(min_level), f64::from(max_level)) as u8
}

impl TileKey {
    pub fn bounds(self) -> TileBounds {
        let span = span_deg(self.level);
        let west = -180.0 + f64::from(self.col) * span;
        let north = 90.0 - f64::from(self.row) * span;
        let east = (west + span).min(180.0);
        let south = (north - span).max(-90.0);
        TileBounds {
            west,
            east,
            south,
            north,
            filled: [(east - west) / span, (north - south) / span],
        }
    }

    /// The tile one level up that this tile lies inside, or `None` at level 0.
    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| Self {
            level: self.level - 1,
            row: self.row / 2,
            col: self.col / 2,
        })
    }

    /// The tile at `level` that contains the given place.
    pub fn containing(level: u8, latitude_deg: f64, longitude_deg: f64) -> Self {
        let span = span_deg(level);
        let (cols, rows) = grid_size(level);
        let longitude = (longitude_deg + 180.0).rem_euclid(360.0);
        let row = ((90.0 - latitude_deg.clamp(-90.0, 90.0)) / span).floor() as u32;
        let col = (longitude / span).floor() as u32;
        Self {
            level,
            row: row.min(rows - 1),
            col: col.min(cols - 1),
        }
    }
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
    let (cols, rows) = grid_size(level);
    let north = (latitude_deg + radius_deg).min(90.0);
    let south = (latitude_deg - radius_deg).max(-90.0);
    let first_row = TileKey::containing(level, north, 0.0).row;
    let last_row = TileKey::containing(level, south, 0.0).row.min(rows - 1);

    // How far east and west the circle reaches. Around a pole it reaches every longitude.
    let reaches_pole = north >= 90.0 || south <= -90.0 || radius_deg >= 90.0;
    let half_width = if reaches_pole {
        180.0
    } else {
        let ratio = radius_deg.to_radians().sin() / latitude_deg.to_radians().cos();
        ratio.clamp(-1.0, 1.0).asin().to_degrees()
    };

    let column_ranges: Vec<(u32, u32)> = if half_width >= 180.0 {
        vec![(0, cols - 1)]
    } else {
        let west = TileKey::containing(level, 0.0, longitude_deg - half_width).col;
        let east = TileKey::containing(level, 0.0, longitude_deg + half_width).col;
        if west <= east {
            vec![(west, east)]
        } else {
            // The circle straddles longitude 180, where the columns start again.
            vec![(west, cols - 1), (0, east)]
        }
    };

    let mut tiles = Vec::new();
    for row in first_row..=last_row {
        for &(first_col, last_col) in &column_ranges {
            for col in first_col..=last_col {
                tiles.push(TileKey { level, row, col });
            }
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_matches_the_sizes_the_service_publishes() {
        assert_eq!(grid_size(0), (2, 1));
        assert_eq!(grid_size(1), (3, 2));
        assert_eq!(grid_size(7), (160, 80));
        assert_eq!(grid_size(11), (2560, 1280));
        assert_eq!(span_deg(7), 2.25);
    }

    #[test]
    fn a_place_is_inside_the_tile_that_contains_it() {
        // San Francisco. Tile 9/92/102 was fetched from the service and shows the bay.
        let tile = TileKey::containing(9, 37.77, -122.42);
        assert_eq!(
            tile,
            TileKey {
                level: 9,
                row: 92,
                col: 102
            }
        );

        let bounds = tile.bounds();
        assert!(bounds.west <= -122.42 && -122.42 < bounds.east);
        assert!(bounds.south <= 37.77 && 37.77 < bounds.north);
        assert_eq!(bounds.filled, [1.0, 1.0]);
        assert!((bounds.east - bounds.west - 0.5625).abs() < 1e-12);
    }

    #[test]
    fn edge_tiles_stop_at_the_edge_of_the_world() {
        // Level 0 tiles are 288 degrees across, so the second one overhangs by 216.
        let east = TileKey {
            level: 0,
            row: 0,
            col: 1,
        }
        .bounds();
        assert_eq!((east.west, east.east), (108.0, 180.0));
        assert_eq!(east.filled[0], 0.25);
        // And both overhang the South Pole.
        assert_eq!((east.north, east.south), (90.0, -90.0));
        assert_eq!(east.filled[1], 0.625);
    }

    #[test]
    fn places_at_the_very_edges_still_have_a_tile() {
        let (cols, rows) = grid_size(5);
        assert_eq!(TileKey::containing(5, -90.0, 179.999).row, rows - 1);
        assert_eq!(TileKey::containing(5, -90.0, 179.999).col, cols - 1);
        assert_eq!(
            TileKey::containing(5, 90.0, -180.0),
            TileKey {
                level: 5,
                row: 0,
                col: 0
            }
        );
        // Longitude 180 east is the same meridian as 180 west.
        assert_eq!(TileKey::containing(5, 0.0, 180.0).col, 0);
    }

    #[test]
    fn a_tile_lies_inside_its_parent() {
        let tile = TileKey {
            level: 9,
            row: 92,
            col: 102,
        };
        let parent = tile.parent().unwrap();
        assert_eq!(
            parent,
            TileKey {
                level: 8,
                row: 46,
                col: 51
            }
        );
        let (inner, outer) = (tile.bounds(), parent.bounds());
        assert!(outer.west <= inner.west && inner.east <= outer.east);
        assert!(outer.south <= inner.south && inner.north <= outer.north);
        assert_eq!(
            TileKey {
                level: 0,
                row: 0,
                col: 0
            }
            .parent(),
            None
        );
    }

    #[test]
    fn the_level_is_the_deepest_that_is_not_finer_than_the_screen() {
        // Level 0 is 0.5625 degrees per pixel, and each level halves that.
        assert_eq!(level_for_resolution(0.5625, 0, 11), 0);
        assert_eq!(level_for_resolution(0.07, 0, 11), 3);
        assert_eq!(level_for_resolution(0.000_3, 0, 11), 10);
        // Out-of-range answers are clamped.
        assert_eq!(level_for_resolution(5.0, 2, 11), 2);
        assert_eq!(level_for_resolution(1e-9, 2, 11), 11);
    }

    #[test]
    fn covering_a_small_circle_takes_a_few_tiles_around_it() {
        let tiles = tiles_covering(37.77, -122.42, 1.0, 9);
        assert!(tiles.contains(&TileKey::containing(9, 37.77, -122.42)));
        assert!(tiles.contains(&TileKey::containing(9, 38.7, -122.42)));
        assert!(tiles.contains(&TileKey::containing(9, 37.77, -121.2)));
        assert!((9..=30).contains(&tiles.len()), "{} tiles", tiles.len());
    }

    #[test]
    fn covering_wraps_across_longitude_180() {
        let tiles = tiles_covering(0.0, 179.0, 3.0, 6);
        let (cols, _) = grid_size(6);
        assert!(tiles.iter().any(|tile| tile.col == 0));
        assert!(tiles.iter().any(|tile| tile.col == cols - 1));
        // Nothing from the far side of the world.
        assert!(tiles.iter().all(|tile| tile.col < 2 || tile.col > cols - 3));
    }

    #[test]
    fn covering_a_pole_takes_every_column_near_it() {
        let tiles = tiles_covering(88.0, 10.0, 5.0, 4);
        let (cols, _) = grid_size(4);
        let top_row = tiles.iter().filter(|tile| tile.row == 0).count() as u32;
        assert_eq!(top_row, cols);
        // A hemisphere seen from far away covers every column too.
        let half = tiles_covering(20.0, 0.0, 80.0, 2);
        assert_eq!(half.len() as u32, grid_size(2).0 * grid_size(2).1);
    }
}
