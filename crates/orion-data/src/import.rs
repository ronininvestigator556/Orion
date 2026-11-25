//! Reading a file of places the user opens: GeoJSON, KML, or CSV with latitude and
//! longitude columns. The file is read where it is and goes nowhere else.

use std::path::Path;

use orion_core::feature::Feature;
use serde_json::Value;

use crate::listing::ParseError;

/// The most Features taken from one file. More than this and the file is not a list
/// of places to look at but a dataset to be prepared some other way.
pub const MAX_IMPORTED: usize = 500_000;
/// The most properties of a Feature shown on its card.
const MAX_ROWS: usize = 40;

/// Reads a file of places, choosing how by its name and, failing that, its contents.
pub fn parse_import(file_name: &str, text: &str) -> Result<Vec<Feature>, ParseError> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase();
    let start = text.trim_start();
    let features = match extension.as_str() {
        "geojson" | "json" => parse_geojson(text),
        "kml" => parse_kml(text),
        "csv" | "tsv" | "txt" => parse_delimited(text),
        "kmz" => Err(ParseError(
            "a KMZ file is a zipped KML file: unzip it and open the KML inside".to_owned(),
        )),
        _ if start.starts_with('{') => parse_geojson(text),
        _ if start.starts_with('<') => parse_kml(text),
        _ => parse_delimited(text),
    }?;
    if features.is_empty() {
        return Err(ParseError(
            "no places with coordinates were found".to_owned(),
        ));
    }
    Ok(features)
}

fn attribute(feature: &mut Feature, label: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() || feature.attributes.lines().count() >= MAX_ROWS {
        return;
    }
    // One row a line, label and value parted by a tab, so neither may hold either.
    let clean = |text: &str| text.replace(['\t', '\n', '\r'], " ");
    feature.attributes.push_str(&clean(label));
    feature.attributes.push('\t');
    feature.attributes.push_str(&clean(value));
    feature.attributes.push('\n');
}

/// Where a place is, as longitude then latitude, and its lines if it has any.
type Place = ([f64; 2], Vec<Vec<[f64; 2]>>);

/// The middle of a shape's points, good enough to stand a dot or a name on.
fn middle(shape: &[Vec<[f64; 2]>]) -> Option<[f64; 2]> {
    let points: Vec<[f64; 2]> = shape.iter().flatten().copied().collect();
    let first = points.first()?;
    // Measured from the first point, so a shape across the date line stays together.
    let (mut east, mut north) = (0.0, 0.0);
    for point in &points {
        east += (point[0] - first[0] + 540.0).rem_euclid(360.0) - 180.0;
        north += point[1] - first[1];
    }
    let count = points.len() as f64;
    Some([
        (first[0] + east / count + 540.0).rem_euclid(360.0) - 180.0,
        first[1] + north / count,
    ])
}

fn point(value: &Value) -> Option<[f64; 2]> {
    Some([value.get(0)?.as_f64()?, value.get(1)?.as_f64()?])
}

fn line(value: &Value) -> Vec<[f64; 2]> {
    value
        .as_array()
        .map(|points| points.iter().filter_map(point).collect())
        .unwrap_or_default()
}

fn lines(value: &Value) -> Vec<Vec<[f64; 2]>> {
    value
        .as_array()
        .map(|lines| lines.iter().map(line).collect())
        .unwrap_or_default()
}

/// A GeoJSON geometry as single points and as lines. Areas become their outlines.
fn geometry(value: &Value, points: &mut Vec<[f64; 2]>, shape: &mut Vec<Vec<[f64; 2]>>) {
    let coordinates = &value["coordinates"];
    match value["type"].as_str() {
        Some("Point") => points.extend(point(coordinates)),
        Some("MultiPoint") => points.extend(line(coordinates)),
        Some("LineString") => shape.push(line(coordinates)),
        Some("MultiLineString" | "Polygon") => shape.extend(lines(coordinates)),
        Some("MultiPolygon") => {
            for polygon in coordinates.as_array().into_iter().flatten() {
                shape.extend(lines(polygon));
            }
        }
        Some("GeometryCollection") => {
            for part in value["geometries"].as_array().into_iter().flatten() {
                geometry(part, points, shape);
            }
        }
        _ => {}
    }
    shape.retain(|line| line.len() >= 2);
}

/// Reads GeoJSON: a collection of features, one feature, or a bare geometry.
pub fn parse_geojson(text: &str) -> Result<Vec<Feature>, ParseError> {
    let document: Value =
        serde_json::from_str(text).map_err(|error| ParseError(error.to_string()))?;
    let listed: Vec<&Value> = match document["type"].as_str() {
        Some("FeatureCollection") => document["features"]
            .as_array()
            .map(|features| features.iter().collect())
            .unwrap_or_default(),
        Some(_) => vec![&document],
        None => return Err(ParseError("not GeoJSON".to_owned())),
    };

    let mut features = Vec::new();
    for (number, entry) in listed.into_iter().enumerate() {
        // A bare geometry stands for itself; a feature carries one.
        let shape_of = if entry["type"].as_str() == Some("Feature") {
            &entry["geometry"]
        } else {
            entry
        };
        let (mut points, mut shape) = (Vec::new(), Vec::new());
        geometry(shape_of, &mut points, &mut shape);
        let properties = entry["properties"].as_object();
        let named = properties.and_then(|properties| {
            ["name", "Name", "NAME", "title", "Title", "label", "id"]
                .into_iter()
                .find_map(|key| match properties.get(key)? {
                    Value::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
        });
        let name = named.unwrap_or_else(|| format!("Place {}", number + 1));

        // Each single point is a place of its own; lines and outlines are one together.
        let mut places: Vec<Place> = points.into_iter().map(|at| (at, Vec::new())).collect();
        if let Some(at) = middle(&shape) {
            places.push((at, shape));
        }
        for (part, (at, shape)) in places.into_iter().enumerate() {
            let mut feature = Feature::at(&format!("{number}/{part}"), name.clone(), at[1], at[0]);
            feature.shape = shape;
            for (key, value) in properties.into_iter().flatten() {
                let value = match value {
                    Value::String(text) => text.clone(),
                    Value::Null | Value::Object(_) | Value::Array(_) => continue,
                    other => other.to_string(),
                };
                attribute(&mut feature, key, &value);
            }
            if feature.is_on_earth() {
                features.push(feature);
            }
            if features.len() >= MAX_IMPORTED {
                return Ok(features);
            }
        }
    }
    Ok(features)
}

/// Text inside the first `<tag>` of `xml`, with markup decoded.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let start = from + xml[from..].find(&open)?;
        let after = &xml[start + open.len()..];
        // `<name>` must not match `<namespace>`.
        if after.starts_with(['>', ' ', '\t', '\n', '\r']) {
            let body = &after[after.find('>')? + 1..];
            return Some(&body[..body.find(&format!("</{tag}>"))?]);
        }
        from = start + open.len();
    }
}

fn decode(text: &str) -> String {
    let text = text.trim();
    let text = text
        .strip_prefix("<![CDATA[")
        .and_then(|inner| inner.strip_suffix("]]>"))
        .unwrap_or(text);
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        // A character written by name or by number, such as `&amp;` or `&#233;`.
        let written = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let character = match &rest[1..end] {
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "amp" => '&',
                number => {
                    let number = number.strip_prefix('#')?;
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hexadecimal) => u32::from_str_radix(hexadecimal, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((character, end))
        });
        match written {
            Some((character, end)) => {
                decoded.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// A KML list of coordinates: longitude, latitude and maybe height, comma-separated,
/// with spaces between points.
fn kml_coordinates(text: &str) -> Vec<[f64; 2]> {
    text.split_whitespace()
        .filter_map(|point| {
            let mut numbers = point.split(',').map(|number| number.parse::<f64>());
            Some([numbers.next()?.ok()?, numbers.next()?.ok()?])
        })
        .collect()
}

/// Reads the placemarks of a KML file: points, paths and the outlines of areas.
pub fn parse_kml(text: &str) -> Result<Vec<Feature>, ParseError> {
    if !text.contains("<kml") && !text.contains("<Placemark") {
        return Err(ParseError("not KML".to_owned()));
    }
    let mut features = Vec::new();
    let mut rest = text;
    let mut number = 0;
    while let Some(start) = rest.find("<Placemark") {
        let Some(length) = rest[start..].find("</Placemark>") else {
            break;
        };
        let placemark = &rest[start..start + length];
        rest = &rest[start + length..];
        number += 1;

        let (mut points, mut shape) = (Vec::new(), Vec::new());
        let mut within = placemark;
        while let Some(at) = within.find("<coordinates") {
            let Some(listed) = element(&within[at..], "coordinates") else {
                break;
            };
            // What the coordinates belong to is named by the nearest tag opened before them.
            let before = &within[..at];
            let is_point =
                before.rfind("<Point") > before.rfind("<Line").max(before.rfind("<Polygon"));
            let listed_points = kml_coordinates(listed);
            if is_point {
                points.extend(listed_points);
            } else if listed_points.len() >= 2 {
                shape.push(listed_points);
            }
            within = &within[at + "<coordinates".len()..];
        }

        let name = element(placemark, "name")
            .map(decode)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Place {number}"));
        let description = element(placemark, "description").map(decode);
        let mut places: Vec<Place> = points.into_iter().map(|at| (at, Vec::new())).collect();
        if let Some(at) = middle(&shape) {
            places.push((at, shape));
        }
        for (part, (at, shape)) in places.into_iter().enumerate() {
            let mut feature = Feature::at(&format!("{number}/{part}"), name.clone(), at[1], at[0]);
            feature.shape = shape;
            if let Some(description) = description.as_deref().filter(|text| !text.is_empty()) {
                feature
                    .details
                    .push(description.chars().take(1_000).collect());
            }
            if feature.is_on_earth() {
                features.push(feature);
            }
        }
        if features.len() >= MAX_IMPORTED {
            break;
        }
    }
    Ok(features)
}

/// Reads a table with a heading row, separated by commas, tabs or semicolons, that
/// has a column for latitude and one for longitude. Rows without both are skipped.
pub fn parse_delimited(text: &str) -> Result<Vec<Feature>, ParseError> {
    let heading = text.lines().next().unwrap_or("");
    let delimiter = b"\t;,"
        .iter()
        .copied()
        .max_by_key(|delimiter| heading.bytes().filter(|byte| byte == delimiter).count())
        .unwrap_or(b',');
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|error| ParseError(error.to_string()))?
        .clone();
    let column = |names: &[&str]| {
        headers.iter().position(|title| {
            let title = title.trim().trim_start_matches('\u{feff}').to_lowercase();
            names.contains(&title.as_str())
        })
    };
    let latitude = column(&["latitude", "lat", "y", "latitude_deg", "lat_deg"]);
    let longitude = column(&[
        "longitude",
        "lon",
        "lng",
        "long",
        "x",
        "longitude_deg",
        "lon_deg",
    ]);
    let (Some(latitude), Some(longitude)) = (latitude, longitude) else {
        return Err(ParseError(
            "no latitude and longitude columns: name them latitude and longitude, or lat and lon"
                .to_owned(),
        ));
    };
    let name = column(&["name", "title", "label", "place", "id"]);

    let mut features = Vec::new();
    for (number, record) in reader.records().enumerate() {
        let Ok(record) = record else { continue };
        let number_at = |index: usize| record.get(index)?.trim().parse::<f64>().ok();
        let (Some(latitude_deg), Some(longitude_deg)) = (number_at(latitude), number_at(longitude))
        else {
            continue;
        };
        let title = name
            .and_then(|index| record.get(index))
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(|| format!("Row {}", number + 2), str::to_owned);
        let mut feature = Feature::at(&number.to_string(), title, latitude_deg, longitude_deg);
        for (index, value) in record.iter().enumerate() {
            if index != latitude && index != longitude && Some(index) != name {
                attribute(&mut feature, headers.get(index).unwrap_or(""), value);
            }
        }
        if feature.is_on_earth() {
            features.push(feature);
        }
        if features.len() >= MAX_IMPORTED {
            break;
        }
    }
    Ok(features)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEOJSON: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"name":"Depot","capacity":120,"open":true,"notes":null,"tags":["a"]},
         "geometry":{"type":"Point","coordinates":[-0.12,51.5]}},
        {"type":"Feature","properties":{"title":"Route 7"},
         "geometry":{"type":"LineString","coordinates":[[0,0],[2,0],[2,2]]}},
        {"type":"Feature","properties":{},
         "geometry":{"type":"Polygon","coordinates":[[[10,10],[12,10],[12,12],[10,12],[10,10]]]}},
        {"type":"Feature","properties":{"name":"Three stops"},
         "geometry":{"type":"MultiPoint","coordinates":[[1,1],[2,2],[3,3]]}},
        {"type":"Feature","properties":{"name":"Nowhere"},"geometry":null},
        {"type":"Feature","properties":{"name":"Off the map"},
         "geometry":{"type":"Point","coordinates":[500,95]}}]}"#;

    #[test]
    fn geojson_gives_points_routes_and_outlines() {
        let features = parse_import("places.geojson", GEOJSON).unwrap();
        let names: Vec<&str> = features
            .iter()
            .map(|feature| feature.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Depot",
                "Route 7",
                "Place 3",
                "Three stops",
                "Three stops",
                "Three stops"
            ]
        );
        let depot = &features[0];
        assert_eq!((depot.latitude_deg, depot.longitude_deg), (51.5, -0.12));
        assert!(depot.shape.is_empty());
        // Numbers and yes-or-no values are shown, in alphabetical order; lists, nothing
        // and nested things are not.
        assert_eq!(depot.attributes, "capacity\t120\nname\tDepot\nopen\ttrue\n");

        let route = &features[1];
        assert_eq!(route.shape, [vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]]);
        assert!((route.longitude_deg - 4.0 / 3.0).abs() < 1e-9);

        // An area is its outline, with its dot in the middle.
        let area = &features[2];
        assert_eq!(area.shape[0].len(), 5);
        assert!((area.latitude_deg - 10.8).abs() < 1e-9);

        // Each has a key of its own.
        let mut keys: Vec<u64> = features.iter().map(|feature| feature.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), features.len());
    }

    #[test]
    fn a_single_geojson_feature_or_bare_geometry_is_read_too() {
        let one = r#"{"type":"Feature","properties":{"name":"Only"},"geometry":{"type":"Point","coordinates":[5,6]}}"#;
        assert_eq!(parse_import("one.json", one).unwrap()[0].name, "Only");
        let bare = r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#;
        let features = parse_import("unnamed", bare).unwrap();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].shape.len(), 1);
    }

    const KML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
        <kml xmlns="http://www.opengis.net/kml/2.2"><Document><name>My places</name>
        <Placemark><name>Caf&#233; &amp; Bar</name><description><![CDATA[Open <b>late</b>]]></description>
          <Point><coordinates>-122.0822,37.4222,0</coordinates></Point></Placemark>
        <Placemark><name>Trail</name>
          <LineString><coordinates>
            -112.1,36.1,0 -112.2,36.2,0
            -112.3,36.3,0
          </coordinates></LineString></Placemark>
        <Placemark>
          <Polygon><outerBoundaryIs><LinearRing><coordinates>0,0 1,0 1,1 0,0</coordinates></LinearRing></outerBoundaryIs></Polygon>
        </Placemark>
        </Document></kml>"#;

    #[test]
    fn kml_placemarks_are_read() {
        let features = parse_import("places.kml", KML).unwrap();
        assert_eq!(features.len(), 3);
        // The document's own name is not mistaken for a placemark's.
        assert_eq!(features[0].name, "Café & Bar");
        assert_eq!(features[0].details, ["Open <b>late</b>"]);
        assert_eq!(
            (features[0].latitude_deg, features[0].longitude_deg),
            (37.4222, -122.0822)
        );
        assert_eq!(features[1].name, "Trail");
        assert_eq!(features[1].shape[0].len(), 3);
        assert_eq!(features[2].name, "Place 3");
        assert_eq!(features[2].shape[0].len(), 4);
    }

    #[test]
    fn characters_written_by_name_or_number_are_decoded() {
        assert_eq!(
            decode("Caf&#233; &amp; Bar &#x263A; &lt;3"),
            "Café & Bar ☺ <3"
        );
        assert_eq!(
            decode("AT&T & co; 50% &unknown; &#zz;"),
            "AT&T & co; 50% &unknown; &#zz;"
        );
        assert_eq!(decode("  <![CDATA[a < b & c]]> "), "a < b & c");
    }

    #[test]
    fn tables_need_latitude_and_longitude_columns() {
        let csv = "Name,Latitude,Longitude,Kind,Notes\n\
            Alpha,51.5,-0.12,Depot,\"Big, and busy\"\n\
            ,48.85,2.35,Office,\n\
            Bad,north,east,,\n\
            Far,95,10,,\n";
        let features = parse_import("sites.csv", csv).unwrap();
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].name, "Alpha");
        assert_eq!(
            features[0].attributes,
            "Kind\tDepot\nNotes\tBig, and busy\n"
        );
        // No name: the row's number in the file stands in.
        assert_eq!(features[1].name, "Row 3");

        // Semicolons and short column names, as spreadsheets in much of Europe write them.
        let semicolons = "lat;lon;label\n10;20;Here\n";
        let features = parse_import("x.csv", semicolons).unwrap();
        assert_eq!(features[0].name, "Here");
        assert_eq!(
            (features[0].latitude_deg, features[0].longitude_deg),
            (10.0, 20.0)
        );

        let error = parse_import("x.csv", "name,address\nA,1 High Street\n").unwrap_err();
        assert!(error.0.contains("latitude and longitude"), "{error}");
    }

    #[test]
    fn files_with_nothing_to_show_are_errors() {
        assert!(
            parse_import(
                "empty.geojson",
                r#"{"type":"FeatureCollection","features":[]}"#
            )
            .is_err()
        );
        assert!(parse_import("x.kml", "<kml></kml>").is_err());
        assert!(parse_import("x.kmz", "PK").unwrap_err().0.contains("unzip"));
        assert!(parse_import("x.json", "not json").is_err());
    }

    #[test]
    fn a_shape_across_the_date_line_has_its_middle_there() {
        let at = middle(&[vec![[179.0, 10.0], [-179.0, 12.0]]]).unwrap();
        assert!(at[0].abs() > 179.9, "{}", at[0]);
        assert!((at[1] - 11.0).abs() < 1e-9);
    }
}
