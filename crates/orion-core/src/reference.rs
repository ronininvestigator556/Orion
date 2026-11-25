//! The lines and names drawn over the Base Map so a place can be recognised: borders,
//! major roads, countries and cities. They are bundled with the app in two compact
//! files, which this module reads and writes.

use std::fmt;

/// What a line over the Base Map marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LineKind {
    /// A border between countries.
    Border,
    /// A border that is disputed, undefined or otherwise not settled.
    UnsettledBorder,
    /// A border between the states or provinces of one country.
    StateLine,
    MajorRoad,
    Road,
}

impl LineKind {
    pub const ALL: [Self; 5] = [
        Self::Border,
        Self::UnsettledBorder,
        Self::StateLine,
        Self::MajorRoad,
        Self::Road,
    ];

    fn code(self) -> u8 {
        match self {
            Self::Border => 0,
            Self::UnsettledBorder => 1,
            Self::StateLine => 2,
            Self::MajorRoad => 3,
            Self::Road => 4,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

/// One line over the Base Map.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceLine {
    pub kind: LineKind,
    /// Longitude then latitude of each point in degrees, east and north positive.
    pub points: Vec<[f64; 2]>,
}

/// Why a file of lines could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceError {
    NotAReferenceFile,
    /// The file ends before the lines it promises.
    CutShort,
    UnknownKind(u8),
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAReferenceFile => write!(f, "not a file of Base Map lines"),
            Self::CutShort => write!(f, "the file of Base Map lines is cut short"),
            Self::UnknownKind(code) => write!(f, "unknown kind of Base Map line, {code}"),
        }
    }
}

impl std::error::Error for ReferenceError {}

const MAGIC: &[u8; 4] = b"ORL1";
/// Stored units per degree: a step of about 11 metres on the ground.
const UNITS_PER_DEGREE: f64 = 10_000.0;
/// The most points one stored line can hold.
const MAX_POINTS: usize = u16::MAX as usize;

fn to_units(point: [f64; 2]) -> [i32; 2] {
    point.map(|degrees| (degrees * UNITS_PER_DEGREE).round() as i32)
}

/// Packs lines into the bundled format: each line stores its first point in full and
/// every later point as a small step from the one before, four bytes a point.
pub fn encode_reference_lines(lines: &[ReferenceLine]) -> Vec<u8> {
    // Steps that do not fit are split with points in between, and lines too long for
    // one count are cut into pieces that share an end point.
    let mut pieces: Vec<(LineKind, Vec<[i32; 2]>)> = Vec::new();
    for line in lines {
        let mut units: Vec<[i32; 2]> = Vec::with_capacity(line.points.len());
        for point in line.points.iter().copied().map(to_units) {
            if let Some(&previous) = units.last() {
                if previous == point {
                    continue;
                }
                let widest = (point[0] - previous[0])
                    .abs()
                    .max((point[1] - previous[1]).abs());
                let steps = widest.unsigned_abs().div_ceil(i16::MAX as u32) as i32;
                for step in 1..steps {
                    units.push([0, 1].map(|axis| {
                        previous[axis]
                            + ((i64::from(point[axis] - previous[axis]) * i64::from(step))
                                / i64::from(steps)) as i32
                    }));
                }
            }
            units.push(point);
        }
        let mut rest = units.as_slice();
        while rest.len() >= 2 {
            let take = rest.len().min(MAX_POINTS);
            pieces.push((line.kind, rest[..take].to_vec()));
            // The next piece starts where this one ended.
            rest = &rest[take - 1..];
        }
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(pieces.len() as u32).to_le_bytes());
    for (kind, points) in pieces {
        bytes.push(kind.code());
        bytes.push(0);
        bytes.extend_from_slice(&(points.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&points[0][0].to_le_bytes());
        bytes.extend_from_slice(&points[0][1].to_le_bytes());
        for pair in points.windows(2) {
            for axis in [0, 1] {
                let step = (pair[1][axis] - pair[0][axis]) as i16;
                bytes.extend_from_slice(&step.to_le_bytes());
            }
        }
    }
    bytes
}

/// Reads lines packed by [`encode_reference_lines`].
pub fn parse_reference_lines(bytes: &[u8]) -> Result<Vec<ReferenceLine>, ReferenceError> {
    let mut reader = Reader { bytes };
    if reader.take::<4>().ok() != Some(*MAGIC) {
        return Err(ReferenceError::NotAReferenceFile);
    }
    let count = u32::from_le_bytes(reader.take()?);
    let mut lines = Vec::new();
    for _ in 0..count {
        let [code, _reserved] = reader.take()?;
        let kind = LineKind::from_code(code).ok_or(ReferenceError::UnknownKind(code))?;
        let point_count = usize::from(u16::from_le_bytes(reader.take()?));
        if point_count == 0 {
            return Err(ReferenceError::CutShort);
        }
        let mut units = [
            i32::from_le_bytes(reader.take()?),
            i32::from_le_bytes(reader.take()?),
        ];
        let mut points = Vec::with_capacity(point_count);
        points.push(units.map(|unit| f64::from(unit) / UNITS_PER_DEGREE));
        for _ in 1..point_count {
            for unit in &mut units {
                *unit += i32::from(i16::from_le_bytes(reader.take()?));
            }
            points.push(units.map(|unit| f64::from(unit) / UNITS_PER_DEGREE));
        }
        lines.push(ReferenceLine { kind, points });
    }
    Ok(lines)
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ReferenceError> {
        let (head, rest) = self
            .bytes
            .split_first_chunk::<N>()
            .ok_or(ReferenceError::CutShort)?;
        self.bytes = rest;
        Ok(*head)
    }
}

/// Removes points a line can do without: the result never strays more than
/// `tolerance_deg` from the original. The first and last points are always kept.
pub fn simplify(points: &[[f64; 2]], tolerance_deg: f64) -> Vec<[f64; 2]> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((first, last)) = spans.pop() {
        let mut farthest = (0.0, first);
        for index in first + 1..last {
            let distance = distance_from_segment(points[index], points[first], points[last]);
            if distance > farthest.0 {
                farthest = (distance, index);
            }
        }
        if farthest.0 > tolerance_deg {
            keep[farthest.1] = true;
            spans.push((first, farthest.1));
            spans.push((farthest.1, last));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(point, keep)| keep.then_some(*point))
        .collect()
}

/// Distance in degrees from `point` to the straight piece from `start` to `end`, with
/// longitude shrunk by latitude so that a degree means the same in every direction.
fn distance_from_segment(point: [f64; 2], start: [f64; 2], end: [f64; 2]) -> f64 {
    let shrink = point[1].to_radians().cos();
    let to = |other: [f64; 2]| [(other[0] - start[0]) * shrink, other[1] - start[1]];
    let (along, offset) = (to(end), to(point));
    let length_squared = along[0] * along[0] + along[1] * along[1];
    let t = if length_squared == 0.0 {
        0.0
    } else {
        ((offset[0] * along[0] + offset[1] * along[1]) / length_squared).clamp(0.0, 1.0)
    };
    (offset[0] - t * along[0]).hypot(offset[1] - t * along[1])
}

/// What a name over the Base Map belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Country,
    /// The capital city of a country.
    Capital,
    City,
}

/// A name drawn over the Base Map.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceName {
    pub kind: PlaceKind,
    /// How prominent the place is: 0 for the most, rising for lesser ones.
    pub rank: u8,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub name: String,
}

/// Reads place names from tab-separated lines of kind, rank, latitude, longitude and
/// name, where kind is `N` for a country, `C` for a capital and `P` for any other city.
/// Lines that cannot be read are skipped.
pub fn parse_place_names(text: &str) -> Vec<PlaceName> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let kind = match fields.next()? {
                "N" => PlaceKind::Country,
                "C" => PlaceKind::Capital,
                "P" => PlaceKind::City,
                _ => return None,
            };
            let rank = fields.next()?.parse().ok()?;
            let latitude_deg = fields.next()?.parse().ok()?;
            let longitude_deg = fields.next()?.parse().ok()?;
            let name = fields.next()?.trim();
            (!name.is_empty()).then(|| PlaceName {
                kind,
                rank,
                latitude_deg,
                longitude_deg,
                name: name.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(a, b)| (a[0] - b[0]).abs() < 6e-5 && (a[1] - b[1]).abs() < 6e-5)
    }

    #[test]
    fn lines_survive_packing_to_within_a_few_metres() {
        let lines = vec![
            ReferenceLine {
                kind: LineKind::Border,
                points: vec![[-122.41941, 37.77493], [-122.3, 37.8], [-121.9, 37.33]],
            },
            ReferenceLine {
                kind: LineKind::Road,
                points: vec![[179.99, -45.0], [178.5, -44.25]],
            },
        ];
        let read = parse_reference_lines(&encode_reference_lines(&lines)).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].kind, LineKind::Border);
        assert_eq!(read[1].kind, LineKind::Road);
        assert!(close(&read[0].points, &lines[0].points), "{:?}", read[0]);
        assert!(close(&read[1].points, &lines[1].points), "{:?}", read[1]);
    }

    #[test]
    fn a_step_too_long_to_store_gains_points_in_between() {
        // 40 degrees in one step is twelve times what a stored step can hold.
        let line = ReferenceLine {
            kind: LineKind::StateLine,
            points: vec![[-100.0, 49.0], [-60.0, 49.0]],
        };
        let read = parse_reference_lines(&encode_reference_lines(&[line])).unwrap();
        let points = &read[0].points;
        assert!(points.len() > 2);
        assert!(close(&[points[0]], &[[-100.0, 49.0]]));
        assert!(close(&[*points.last().unwrap()], &[[-60.0, 49.0]]));
        assert!(
            points
                .windows(2)
                .all(|pair| pair[1][0] > pair[0][0] && (pair[1][1] - 49.0).abs() < 1e-4)
        );
    }

    #[test]
    fn a_line_longer_than_one_count_is_cut_into_joined_pieces() {
        let points: Vec<[f64; 2]> = (0..70_000)
            .map(|step| [f64::from(step) * 0.001, 10.0])
            .collect();
        let line = ReferenceLine {
            kind: LineKind::MajorRoad,
            points: points.clone(),
        };
        let read = parse_reference_lines(&encode_reference_lines(&[line])).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].points.last(), read[1].points.first());
        let total: usize = read.iter().map(|line| line.points.len()).sum();
        assert_eq!(total, points.len() + 1);
    }

    #[test]
    fn single_points_and_repeats_are_dropped() {
        let lines = [
            ReferenceLine {
                kind: LineKind::Border,
                points: vec![[1.0, 1.0]],
            },
            ReferenceLine {
                kind: LineKind::Border,
                points: vec![[2.0, 2.0], [2.0, 2.0], [3.0, 3.0]],
            },
        ];
        let read = parse_reference_lines(&encode_reference_lines(&lines)).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].points.len(), 2);
    }

    #[test]
    fn damaged_files_are_refused() {
        assert_eq!(
            parse_reference_lines(b"nope"),
            Err(ReferenceError::NotAReferenceFile)
        );
        let line = ReferenceLine {
            kind: LineKind::Border,
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        };
        let mut bytes = encode_reference_lines(&[line]);
        bytes.pop();
        assert_eq!(parse_reference_lines(&bytes), Err(ReferenceError::CutShort));
        bytes[8] = 99;
        assert_eq!(
            parse_reference_lines(&bytes),
            Err(ReferenceError::UnknownKind(99))
        );
    }

    #[test]
    fn simplifying_keeps_corners_and_drops_points_on_the_way() {
        let line = [
            [0.0, 0.0],
            [1.0, 0.00001],
            [2.0, 0.0],
            [2.0, 1.0],
            [2.00001, 2.0],
            [2.0, 3.0],
        ];
        assert_eq!(simplify(&line, 0.001), [[0.0, 0.0], [2.0, 0.0], [2.0, 3.0]]);
        // A tolerance of nothing keeps every point that is off the straight line.
        assert_eq!(simplify(&line, 0.0), line);
        assert_eq!(simplify(&line[..2], 1.0), &line[..2]);
    }

    #[test]
    fn place_names_are_read_and_bad_lines_skipped() {
        let text = "N\t2\t46.2\t2.2\tFrance\nC\t0\t48.8667\t2.3333\tParis\nP\t7\t43.3\t5.37\tMarseille\nX\t1\t0\t0\tNowhere\nP\tseven\t0\t0\tBad rank\nP\t3\t1\t1\t\n";
        let places = parse_place_names(text);
        assert_eq!(places.len(), 3);
        assert_eq!(places[0].kind, PlaceKind::Country);
        assert_eq!(places[1].kind, PlaceKind::Capital);
        assert_eq!(places[1].name, "Paris");
        assert_eq!(places[2].rank, 7);
        assert_eq!(places[2].longitude_deg, 5.37);
    }
}
