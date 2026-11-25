//! Builds the lines and names bundled for the Base Map from Natural Earth, a
//! public-domain map data set (naturalearthdata.com).
//!
//! Download these files from the `geojson` folder of
//! <https://github.com/nvkelso/natural-earth-vector> into one folder:
//!
//! - `ne_10m_admin_0_boundary_lines_land.geojson`
//! - `ne_10m_admin_1_states_provinces_lines.geojson`
//! - `ne_10m_roads.geojson`
//! - `ne_10m_populated_places_simple.geojson`
//! - `ne_50m_admin_0_countries.geojson`
//!
//! Then run, from the repository root:
//!
//! ```text
//! cargo run --release -p orion-core --example make_reference -- <folder> crates/orion-app/assets
//! ```

use std::fs;
use std::path::Path;

use orion_core::reference::{LineKind, ReferenceLine, encode_reference_lines, simplify};
use serde_json::Value;

/// How far, in degrees, a simplified line may stray from the original: about 100 metres
/// for borders and 300 for roads, both finer than Natural Earth's own accuracy.
const BORDER_TOLERANCE_DEG: f64 = 0.001;
const ROAD_TOLERANCE_DEG: f64 = 0.003;
/// State lines Natural Earth suggests showing only beyond this zoom are left out.
const MAX_STATE_LINE_ZOOM: f64 = 7.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let (Some(source), Some(destination)) = (arguments.next(), arguments.next()) else {
        return Err("usage: make_reference <folder of Natural Earth files> <assets folder>".into());
    };
    let (source, destination) = (Path::new(&source), Path::new(&destination));

    let mut lines = Vec::new();
    for feature in features(source, "ne_10m_admin_0_boundary_lines_land")? {
        let kind = match text(&feature, "FEATURECLA") {
            class if class.starts_with("International boundary") => LineKind::Border,
            _ => LineKind::UnsettledBorder,
        };
        push_lines(&mut lines, &feature, kind, BORDER_TOLERANCE_DEG);
    }
    let borders = lines.len();
    for feature in features(source, "ne_10m_admin_1_states_provinces_lines")? {
        // Leave out the statistical regions some countries are divided into on paper,
        // and the divisions too small to matter until far closer than Orion's view goes.
        let min_zoom = feature["properties"]["MIN_ZOOM"].as_f64().unwrap_or(99.0);
        if text(&feature, "FEATURECLA") == "Admin-1 boundary" && min_zoom <= MAX_STATE_LINE_ZOOM {
            push_lines(
                &mut lines,
                &feature,
                LineKind::StateLine,
                BORDER_TOLERANCE_DEG,
            );
        }
    }
    let state_lines = lines.len() - borders;
    for feature in features(source, "ne_10m_roads")? {
        let rank = feature["properties"]["scalerank"].as_i64().unwrap_or(99);
        let kind = match text(&feature, "type") {
            "Major Highway" | "Beltway" | "Bypass" => LineKind::MajorRoad,
            "Secondary Highway" => LineKind::Road,
            "Road" if rank <= 4 => LineKind::Road,
            _ => continue,
        };
        push_lines(&mut lines, &feature, kind, ROAD_TOLERANCE_DEG);
    }
    let roads = lines.len() - borders - state_lines;
    let points: usize = lines.iter().map(|line| line.points.len()).sum();
    for kind in LineKind::ALL {
        let of_kind = lines.iter().filter(|line| line.kind == kind);
        println!(
            "{kind:?}: {} points",
            of_kind.map(|line| line.points.len()).sum::<usize>()
        );
    }
    let bytes = encode_reference_lines(&lines);
    println!(
        "{borders} border lines, {state_lines} state lines, {roads} roads; {points} points, {} bytes",
        bytes.len()
    );
    fs::write(destination.join("reference_lines.bin"), bytes)?;

    let mut names = Vec::new();
    for feature in features(source, "ne_50m_admin_0_countries")? {
        let properties = &feature["properties"];
        let (Some(latitude), Some(longitude)) = (
            properties["LABEL_Y"].as_f64(),
            properties["LABEL_X"].as_f64(),
        ) else {
            continue;
        };
        let rank = properties["LABELRANK"].as_i64().unwrap_or(9).clamp(0, 9);
        names.push((
            0,
            rank,
            0,
            row('N', rank, latitude, longitude, text(&feature, "NAME")),
        ));
    }
    for feature in features(source, "ne_10m_populated_places_simple")? {
        let properties = &feature["properties"];
        if !text(&feature, "featurecla")
            .to_ascii_lowercase()
            .contains("populated place")
            && !text(&feature, "featurecla").contains("capital")
        {
            continue;
        }
        let (Some(latitude), Some(longitude)) = (
            properties["latitude"].as_f64(),
            properties["longitude"].as_f64(),
        ) else {
            continue;
        };
        let rank = properties["scalerank"].as_i64().unwrap_or(10).clamp(0, 10);
        let capital = properties["adm0cap"].as_f64().unwrap_or(0.0) > 0.0;
        let population = properties["pop_max"].as_f64().unwrap_or(0.0) as i64;
        let kind = if capital { 'C' } else { 'P' };
        names.push((
            1,
            rank,
            -population,
            row(kind, rank, latitude, longitude, text(&feature, "name")),
        ));
    }
    // Most prominent first, so the app can take names in order until the view is full.
    names.sort();
    let table: String = names.iter().map(|(.., row)| row.as_str()).collect();
    println!("{} names, {} bytes", names.len(), table.len());
    fs::write(destination.join("place_names.tsv"), table)?;
    Ok(())
}

fn row(kind: char, rank: i64, latitude: f64, longitude: f64, name: &str) -> String {
    format!(
        "{kind}\t{rank}\t{latitude:.4}\t{longitude:.4}\t{}\n",
        name.replace('\t', " ")
    )
}

fn features(folder: &Path, name: &str) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let path = folder.join(format!("{name}.geojson"));
    let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut document: Value = serde_json::from_str(&text)?;
    match document["features"].take() {
        Value::Array(features) => Ok(features),
        _ => Err(format!("{} has no features", path.display()).into()),
    }
}

fn text<'a>(feature: &'a Value, property: &str) -> &'a str {
    feature["properties"][property].as_str().unwrap_or("")
}

/// Adds each line of a feature's geometry, simplified.
fn push_lines(lines: &mut Vec<ReferenceLine>, feature: &Value, kind: LineKind, tolerance: f64) {
    let geometry = &feature["geometry"];
    let empty = Vec::new();
    let coordinates = geometry["coordinates"].as_array().unwrap_or(&empty);
    let parts: Vec<&Value> = match geometry["type"].as_str() {
        Some("LineString") => vec![&geometry["coordinates"]],
        Some("MultiLineString") => coordinates.iter().collect(),
        _ => Vec::new(),
    };
    for part in parts {
        let points: Vec<[f64; 2]> = part
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|point| Some([point[0].as_f64()?, point[1].as_f64()?]))
            .collect();
        let points = simplify(&points, tolerance);
        if points.len() >= 2 {
            lines.push(ReferenceLine { kind, points });
        }
    }
}
