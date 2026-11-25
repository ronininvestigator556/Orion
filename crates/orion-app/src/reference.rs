//! Borders, roads and names drawn over the Base Map so a place can be recognised.
//! They come with the app, built from Natural Earth, so showing them fetches nothing.

use std::ops::Range;

use anyhow::Context as _;
use glam::Vec3;
use orion_core::reference::{
    LineKind, PlaceKind, PlaceName, ReferenceLine, parse_place_names, parse_reference_lines,
};

use crate::basemap::{BaseMapChoices, visible_radius};
use crate::lines::{Lines, Segment};
use crate::objects::View;
use crate::picking::PickView;

/// Width of a border or road in logical points.
const WIDTH_POINTS: f32 = 1.2;
/// The lines are sorted into squares this many degrees across, so that only the ones
/// near the view are drawn.
const CELL_DEG: f64 = 10.0;
/// The angle from the middle of a square to its corner, with room for a segment that
/// starts inside and ends outside.
const CELL_REACH_DEG: f32 = 7.6;
/// A straight piece longer than this is drawn as several, so it follows the curve of
/// the globe, and of its parallel of latitude, instead of cutting underneath.
const MAX_STEP_DEG: f64 = 0.25;

/// Linear colour of each kind of line.
fn color(kind: LineKind) -> [f32; 4] {
    match kind {
        LineKind::Border => [1.0, 0.97, 0.9, 0.7],
        LineKind::UnsettledBorder => [1.0, 0.97, 0.9, 0.35],
        LineKind::StateLine => [1.0, 0.97, 0.9, 0.3],
        LineKind::MajorRoad => [1.0, 0.72, 0.2, 0.6],
        LineKind::Road => [1.0, 0.72, 0.2, 0.35],
    }
}

/// The height above the ground, in Earth radii, below which a kind of line appears.
/// From farther away it would only clutter the picture.
fn appears_below(kind: LineKind) -> f32 {
    match kind {
        LineKind::Border | LineKind::UnsettledBorder => f32::INFINITY,
        LineKind::StateLine => 1.0,
        LineKind::MajorRoad => 0.15,
        LineKind::Road => 0.05,
    }
}

fn is_road(kind: LineKind) -> bool {
    matches!(kind, LineKind::MajorRoad | LineKind::Road)
}

/// The point on the unit sphere at a longitude and latitude in degrees.
fn on_globe([longitude_deg, latitude_deg]: [f64; 2]) -> Vec3 {
    let (sin_lat, cos_lat) = latitude_deg.to_radians().sin_cos();
    let (sin_lon, cos_lon) = longitude_deg.to_radians().sin_cos();
    Vec3::new(
        (cos_lat * cos_lon) as f32,
        (cos_lat * sin_lon) as f32,
        sin_lat as f32,
    )
}

/// The lines of one kind in one square of the globe.
struct Bucket {
    kind: LineKind,
    /// Unit vector to the middle of the square.
    centre: Vec3,
    /// Where its segments sit among all the segments.
    range: Range<u32>,
}

/// A kind of line and the column and row of a square.
type BucketKey = (LineKind, (u32, u32));

/// Which square a point falls in, as a column and a row.
fn cell([longitude_deg, latitude_deg]: [f64; 2]) -> (u32, u32) {
    let columns = (360.0 / CELL_DEG) as u32;
    let rows = (180.0 / CELL_DEG) as u32;
    let column = ((longitude_deg + 180.0) / CELL_DEG).floor() as i64;
    let row = ((latitude_deg + 90.0) / CELL_DEG).floor() as i64;
    (
        column.rem_euclid(i64::from(columns)) as u32,
        row.clamp(0, i64::from(rows) - 1) as u32,
    )
}

fn cell_centre((column, row): (u32, u32)) -> Vec3 {
    on_globe([
        (f64::from(column) + 0.5) * CELL_DEG - 180.0,
        (f64::from(row) + 0.5) * CELL_DEG - 90.0,
    ])
}

/// Turns lines into segments sorted by kind and square, with the list of what is where.
fn sort_into_buckets(lines: &[ReferenceLine]) -> (Vec<Segment>, Vec<Bucket>) {
    let mut keyed: Vec<(BucketKey, Segment)> = Vec::new();
    for line in lines {
        let color = color(line.kind);
        for pair in line.points.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let widest = (to[0] - from[0]).abs().max((to[1] - from[1]).abs());
            let steps = (widest / MAX_STEP_DEG).ceil().max(1.0) as u32;
            let at = |step: u32| {
                let t = f64::from(step) / f64::from(steps);
                [
                    from[0] + (to[0] - from[0]) * t,
                    from[1] + (to[1] - from[1]) * t,
                ]
            };
            for step in 0..steps {
                let (start, end) = (at(step), at(step + 1));
                let middle = [(start[0] + end[0]) / 2.0, (start[1] + end[1]) / 2.0];
                keyed.push((
                    (line.kind, cell(middle)),
                    Segment::new(on_globe(start), on_globe(end), color),
                ));
            }
        }
    }
    keyed.sort_by_key(|(key, _)| *key);

    let mut buckets: Vec<Bucket> = Vec::new();
    let mut previous = None;
    for (index, (key, _)) in keyed.iter().enumerate() {
        let index = index as u32;
        if previous == Some(*key) {
            if let Some(bucket) = buckets.last_mut() {
                bucket.range.end = index + 1;
            }
        } else {
            buckets.push(Bucket {
                kind: key.0,
                centre: cell_centre(key.1),
                range: index..index + 1,
            });
            previous = Some(*key);
        }
    }
    let segments = keyed.into_iter().map(|(_, segment)| segment).collect();
    (segments, buckets)
}

/// Borders and roads, held on the GPU and drawn where the view is looking.
pub struct ReferenceLines {
    lines: Lines,
    buckets: Vec<Bucket>,
    /// The segments to draw this frame.
    shown: Vec<Range<u32>>,
}

impl ReferenceLines {
    /// `target_format` must be an sRGB format, because the shader outputs linear colour.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        let bundled = parse_reference_lines(include_bytes!("../assets/reference_lines.bin"))
            .context("reading the bundled Base Map lines")?;
        let (segments, buckets) = sort_into_buckets(&bundled);
        let mut lines = Lines::with_width(device, target_format, WIDTH_POINTS);
        lines.set_segments(device, queue, &segments);
        Ok(Self {
            lines,
            buckets,
            shown: Vec::new(),
        })
    }

    /// Chooses what to draw for this view.
    pub fn update(&mut self, queue: &wgpu::Queue, view: &View, choices: BaseMapChoices) {
        self.lines.set_view(queue, view);
        self.shown.clear();
        let distance = view.camera_position.length().max(1.000_1);
        let altitude = distance - 1.0;
        let below = view.camera_position / distance;
        let aspect = view.viewport[0] as f32 / view.viewport[1].max(1) as f32;
        let reach = visible_radius(distance, aspect) + CELL_REACH_DEG.to_radians();
        let nearest = reach.min(std::f32::consts::PI).cos();
        for bucket in &self.buckets {
            let wanted = if is_road(bucket.kind) {
                choices.roads
            } else {
                choices.borders
            };
            if !wanted
                || altitude > appears_below(bucket.kind)
                || bucket.centre.dot(below) < nearest
            {
                continue;
            }
            // Neighbouring squares of one kind sit side by side: draw them as one.
            match self.shown.last_mut() {
                Some(last) if last.end == bucket.range.start => last.end = bucket.range.end,
                _ => self.shown.push(bucket.range.clone()),
            }
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.lines.draw_ranges(pass, &self.shown);
    }
}

/// The most names drawn at once.
const MAX_LABELS: usize = 70;
/// Rough width of one letter of a name, and height of a line, in logical points, for
/// keeping names from overlapping.
const LETTER_POINTS: f32 = 7.5;
const LINE_POINTS: f32 = 18.0;

/// The least prominent country shown from a height above the ground in Earth radii.
fn country_rank_limit(altitude: f32) -> u8 {
    match altitude {
        a if a > 3.0 => 0,
        a if a > 1.6 => 2,
        a if a > 1.0 => 3,
        a if a > 0.5 => 5,
        _ => u8::MAX,
    }
}

/// The least prominent city shown from a height above the ground in Earth radii,
/// or `None` if the view is too far out for any.
fn city_rank_limit(altitude: f32) -> Option<u8> {
    Some(match altitude {
        a if a > 1.6 => return None,
        a if a > 1.0 => 0,
        a if a > 0.6 => 1,
        a if a > 0.4 => 2,
        a if a > 0.25 => 3,
        a if a > 0.15 => 4,
        a if a > 0.08 => 6,
        a if a > 0.04 => 7,
        a if a > 0.02 => 8,
        _ => u8::MAX,
    })
}

/// A name to draw this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceLabel<'a> {
    /// Where the place is on screen, in logical points.
    pub position: egui::Pos2,
    pub kind: PlaceKind,
    pub name: &'a str,
}

/// The names of countries and cities, most prominent first.
pub struct PlaceNames {
    places: Vec<(PlaceName, Vec3)>,
}

impl PlaceNames {
    pub fn bundled() -> Self {
        Self::new(parse_place_names(include_str!("../assets/place_names.tsv")))
    }

    fn new(places: Vec<PlaceName>) -> Self {
        Self {
            places: places
                .into_iter()
                .map(|place| {
                    let direction = on_globe([place.longitude_deg, place.latitude_deg]);
                    (place, direction)
                })
                .collect(),
        }
    }

    /// The names to draw for a view: the most prominent places in sight that suit how
    /// close the view is, leaving out any that would sit on top of one already chosen
    /// or left of `left_edge` logical points, where the sidebar is.
    pub fn labels(
        &self,
        view: &PickView,
        scale_factor: f32,
        left_edge: f32,
    ) -> Vec<PlaceLabel<'_>> {
        let distance = view.camera_position.length().max(1.000_1);
        let altitude = distance - 1.0;
        let below = view.camera_position / distance;
        let aspect = view.viewport.x / view.viewport.y.max(1.0);
        let nearest = visible_radius(distance, aspect).cos();
        let countries = country_rank_limit(altitude);
        let cities = city_rank_limit(altitude);

        let mut labels: Vec<PlaceLabel<'_>> = Vec::new();
        let mut taken: Vec<egui::Rect> = Vec::new();
        for (place, direction) in &self.places {
            let suits = match place.kind {
                PlaceKind::Country => place.rank <= countries,
                PlaceKind::Capital | PlaceKind::City => {
                    cities.is_some_and(|limit| place.rank <= limit)
                }
            };
            if !suits || direction.dot(below) < nearest {
                continue;
            }
            let Some(pixels) = view.project(*direction) else {
                continue;
            };
            let position = egui::pos2(pixels.x / scale_factor, pixels.y / scale_factor);
            if position.x < left_edge {
                continue;
            }
            let rect = label_rect(position, place);
            if taken.iter().any(|other| other.intersects(rect)) {
                continue;
            }
            taken.push(rect);
            labels.push(PlaceLabel {
                position,
                kind: place.kind,
                name: &place.name,
            });
            if labels.len() >= MAX_LABELS {
                break;
            }
        }
        labels
    }
}

/// Roughly the space a name takes up on screen.
fn label_rect(position: egui::Pos2, place: &PlaceName) -> egui::Rect {
    let width = place.name.chars().count() as f32 * LETTER_POINTS + 16.0;
    match place.kind {
        // A country's name is centred on it.
        PlaceKind::Country => {
            egui::Rect::from_center_size(position, egui::vec2(width * 1.2, LINE_POINTS * 1.2))
        }
        // A city's name sits to the right of its dot.
        PlaceKind::Capital | PlaceKind::City => egui::Rect::from_min_size(
            position - egui::vec2(6.0, LINE_POINTS / 2.0),
            egui::vec2(width + 6.0, LINE_POINTS),
        ),
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::*;
    use crate::camera::OrbitCamera;

    fn line(kind: LineKind, points: &[[f64; 2]]) -> ReferenceLine {
        ReferenceLine {
            kind,
            points: points.to_vec(),
        }
    }

    #[test]
    fn the_bundled_lines_and_names_can_be_read() {
        let lines = parse_reference_lines(include_bytes!("../assets/reference_lines.bin")).unwrap();
        for kind in LineKind::ALL {
            assert!(lines.iter().any(|line| line.kind == kind), "no {kind:?}");
        }
        let (segments, buckets) = sort_into_buckets(&lines);
        assert!(segments.len() > 400_000, "{}", segments.len());
        assert_eq!(
            buckets.last().map(|bucket| bucket.range.end),
            Some(segments.len() as u32)
        );

        let names = PlaceNames::bundled();
        assert!(names.places.len() > 7_000);
        assert!(
            names
                .places
                .iter()
                .any(|(place, _)| place.name == "Paris" && place.kind == PlaceKind::Capital)
        );
    }

    #[test]
    fn segments_are_grouped_by_kind_and_square_without_gaps() {
        let lines = [
            line(LineKind::Road, &[[10.1, 10.1], [10.2, 10.2]]),
            line(
                LineKind::Border,
                &[[10.1, 10.1], [10.2, 10.2], [10.3, 10.1]],
            ),
            line(LineKind::Border, &[[-75.0, -40.0], [-75.1, -40.1]]),
        ];
        let (segments, buckets) = sort_into_buckets(&lines);
        assert_eq!(segments.len(), 4);
        let summary: Vec<_> = buckets
            .iter()
            .map(|bucket| (bucket.kind, bucket.range.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                (LineKind::Border, 0..1),
                (LineKind::Border, 1..3),
                (LineKind::Road, 3..4)
            ]
        );
        // Each square's centre is near the lines in it.
        assert!(buckets[1].centre.dot(on_globe([10.1, 10.1])) > CELL_REACH_DEG.to_radians().cos());
    }

    #[test]
    fn a_long_straight_border_follows_its_parallel() {
        // The 49th parallel across the prairies: one piece in the data, many when drawn.
        let (segments, _) =
            sort_into_buckets(&[line(LineKind::Border, &[[-110.0, 49.0], [-100.0, 49.0]])]);
        assert_eq!(segments.len(), 40);
    }

    fn view_from(latitude_deg: f32, longitude_deg: f32, altitude: f32) -> PickView {
        let mut camera = OrbitCamera::default();
        camera.face(on_globe([
            f64::from(longitude_deg),
            f64::from(latitude_deg),
        ]));
        camera.set_altitude(altitude);
        let viewport = Vec2::new(1280.0, 800.0);
        PickView {
            view_projection: camera.view_projection(viewport.x / viewport.y),
            camera_position: camera.position(),
            viewport,
        }
    }

    fn place(
        kind: PlaceKind,
        rank: u8,
        latitude_deg: f64,
        longitude_deg: f64,
        name: &str,
    ) -> PlaceName {
        PlaceName {
            kind,
            rank,
            latitude_deg,
            longitude_deg,
            name: name.to_owned(),
        }
    }

    #[test]
    fn names_suit_how_close_the_view_is() {
        let names = PlaceNames::new(vec![
            place(PlaceKind::Country, 2, 46.0, 2.0, "France"),
            place(PlaceKind::Capital, 0, 48.87, 2.33, "Paris"),
            place(PlaceKind::City, 7, 48.8, 2.13, "Versailles"),
        ]);
        let shown = |altitude: f32| -> Vec<&str> {
            names
                .labels(&view_from(48.85, 2.25, altitude), 1.0, 0.0)
                .into_iter()
                .map(|label| label.name)
                .collect()
        };
        // From the default distance only the country is named.
        assert_eq!(shown(2.2), ["France"]);
        assert_eq!(shown(1.2), ["France", "Paris"]);
        // Close in, the country's own label point is out of sight and small towns appear.
        assert_eq!(shown(0.03), ["Paris", "Versailles"]);
    }

    #[test]
    fn names_do_not_overlap_or_sit_under_the_sidebar() {
        let names = PlaceNames::new(vec![
            place(PlaceKind::Capital, 0, 48.87, 2.33, "Paris"),
            place(PlaceKind::City, 0, 48.88, 2.34, "Almost Paris"),
            place(PlaceKind::City, 0, 43.3, 5.37, "Marseille"),
        ]);
        let view = view_from(47.0, 3.0, 0.3);
        let all: Vec<&str> = names
            .labels(&view, 1.0, 0.0)
            .into_iter()
            .map(|label| label.name)
            .collect();
        assert_eq!(all, ["Paris", "Marseille"]);
        // With the whole window counted as sidebar, nothing is named.
        assert!(names.labels(&view, 1.0, 1280.0).is_empty());
    }

    #[test]
    fn names_on_the_far_side_are_left_out() {
        let names = PlaceNames::new(vec![place(
            PlaceKind::Country,
            0,
            -41.0,
            174.0,
            "New Zealand",
        )]);
        assert!(
            names
                .labels(&view_from(48.0, 2.0, 2.2), 1.0, 0.0)
                .is_empty()
        );
        assert_eq!(
            names.labels(&view_from(-41.0, 174.0, 2.2), 1.0, 0.0).len(),
            1
        );
    }
}
